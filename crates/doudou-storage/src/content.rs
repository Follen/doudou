//! User identity persistence, video index/detail storage, comment JSONL merge, and checkpoints.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use doudou_core::{
    DoudouError, Result, atomic_write_bytes, atomic_write_json, atomic_write_jsonl, profile_paths,
    read_json, read_jsonl, safe_segment, utc_now,
};
use serde_json::{Map, Value, json};

fn value_as_non_empty_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn relative_display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn merge_comment_value(existing: &Value, incoming: &Value) -> Value {
    match incoming {
        Value::Null => existing.clone(),
        Value::String(s) if s.is_empty() => existing.clone(),
        Value::Object(inc_map) => {
            if let Value::Object(ex_map) = existing {
                let mut merged = ex_map.clone();
                for (key, value) in inc_map {
                    let next = merged
                        .get(key)
                        .map_or_else(|| value.clone(), |cur| merge_comment_value(cur, value));
                    merged.insert(key.clone(), next);
                }
                Value::Object(merged)
            } else {
                incoming.clone()
            }
        }
        // Preserve meaningful falsy values such as `0` and `false`.
        _ => incoming.clone(),
    }
}

/// Merge normalized comments by `cid` while preserving stable input order and meaningful falsy values.
pub fn merge_video_comments(existing: &[Value], current: &[Value]) -> Vec<Value> {
    let mut merged: Vec<Value> = Vec::with_capacity(existing.len() + current.len());
    let mut positions: HashMap<String, usize> = HashMap::new();
    for comment in existing.iter().chain(current.iter()) {
        let Some(cid) = value_as_non_empty_string(comment.get("cid")) else {
            continue;
        };
        if let Some(&index) = positions.get(&cid) {
            merged[index] = merge_comment_value(&merged[index], comment);
        } else {
            positions.insert(cid, merged.len());
            merged.push(comment.clone());
        }
    }
    merged
}

/// Resolve the content storage directory for a Douyin user identity (`sec_uid`, `uid`, or `unique_id`).
pub fn user_directory(
    home: &Path,
    data_root: &Path,
    profile: &str,
    identity: &Value,
) -> Result<PathBuf> {
    let stable = value_as_non_empty_string(identity.get("sec_uid"))
        .or_else(|| value_as_non_empty_string(identity.get("uid")))
        .or_else(|| value_as_non_empty_string(identity.get("unique_id")))
        .ok_or_else(|| {
            DoudouError::new(
                "INVALID_IDENTITY",
                "User identity requires sec_uid, uid, or unique_id.",
            )
        })?;
    Ok(profile_paths(home, data_root, profile)?
        .message
        .join("content")
        .join("users")
        .join(safe_segment(&stable, 128)?))
}

/// Resolve the content storage directory for a video (`aweme_id`).
pub fn video_directory(
    home: &Path,
    data_root: &Path,
    profile: &str,
    aweme_id: &str,
) -> Result<PathBuf> {
    Ok(profile_paths(home, data_root, profile)?
        .message
        .join("content")
        .join("videos")
        .join(safe_segment(aweme_id, 64)?))
}

fn validate_comment_records(records: Vec<Value>, error_code: &str) -> Result<Vec<Value>> {
    for (idx, record) in records.iter().enumerate() {
        let valid = record
            .as_object()
            .and_then(|obj| value_as_non_empty_string(obj.get("cid")))
            .is_some();
        if !valid {
            return Err(DoudouError::new(
                error_code,
                format!("Invalid comment record at line {}.", idx + 1),
            )
            .with_details(json!({ "line": idx + 1 })));
        }
    }
    Ok(records)
}

/// Read a saved comment pagination checkpoint and its partial comments, if present.
pub fn read_comment_checkpoint(
    home: &Path,
    data_root: &Path,
    profile: &str,
    aweme_id: &str,
) -> Result<(Option<Value>, Vec<Value>)> {
    let directory = video_directory(home, data_root, profile, aweme_id)?;
    let checkpoint =
        read_json(&directory.join("comment-checkpoint.json"), true)?.filter(Value::is_object);
    let partial_path = directory.join("comments.partial.jsonl");
    let comments = if checkpoint.is_some() && partial_path.exists() {
        let rows = read_jsonl(&partial_path, false)?;
        validate_comment_records(rows, "INVALID_COMMENT_CHECKPOINT")?
    } else {
        Vec::new()
    };
    Ok((checkpoint, comments))
}

/// Read persisted comments for a video from `comments.jsonl`.
pub fn read_video_comments(
    home: &Path,
    data_root: &Path,
    profile: &str,
    aweme_id: &str,
) -> Result<Vec<Value>> {
    let path = video_directory(home, data_root, profile, aweme_id)?.join("comments.jsonl");
    let rows = read_jsonl(&path, true)?;
    validate_comment_records(rows, "INVALID_COMMENT_RECORD")
}

/// Persist deduplicated partial comments and a comment pagination checkpoint atomically.
pub fn write_comment_checkpoint(
    home: &Path,
    data_root: &Path,
    profile: &str,
    aweme_id: &str,
    checkpoint: &Value,
    comments: &[Value],
) -> Result<Value> {
    let directory = video_directory(home, data_root, profile, aweme_id)?;
    let mut deduped: BTreeMap<String, Value> = BTreeMap::new();
    for item in comments {
        if let Some(cid) = value_as_non_empty_string(item.get("cid")) {
            deduped.insert(cid, item.clone());
        }
    }
    let mut ordered: Vec<(String, Value)> = deduped.into_iter().collect();
    ordered.sort_by(|(left_cid, left), (right_cid, right)| {
        let left_ts = left.get("create_time").and_then(Value::as_i64).unwrap_or(0);
        let right_ts = right
            .get("create_time")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        (left_ts, left_cid.as_str()).cmp(&(right_ts, right_cid.as_str()))
    });

    let comment_ids: Vec<String> = ordered.iter().map(|(cid, _)| cid.clone()).collect();
    let ordered_values: Vec<Value> = ordered.into_iter().map(|(_, val)| val).collect();

    let partial_path = directory.join("comments.partial.jsonl");
    atomic_write_jsonl(&partial_path, &ordered_values)?;

    let mut record = Map::new();
    record.insert("schema".to_owned(), json!("doudou.comment-checkpoint.v1"));
    record.insert("aweme_id".to_owned(), json!(aweme_id));
    if let Some(cp_obj) = checkpoint.as_object() {
        for (k, v) in cp_obj {
            record.insert(k.clone(), v.clone());
        }
    }
    record.insert("comment_ids".to_owned(), json!(comment_ids));
    record.insert("comment_count".to_owned(), json!(ordered_values.len()));
    record.insert("updated_at".to_owned(), json!(utc_now()));

    let checkpoint_path = directory.join("comment-checkpoint.json");
    atomic_write_json(&checkpoint_path, &Value::Object(record), None)?;
    Ok(json!({
        "checkpoint": relative_display(&checkpoint_path, data_root),
        "partial_comments": relative_display(&partial_path, data_root),
    }))
}

/// Persist a resolved user identity and its candidate matches under `content/users/<id>/identity.json`.
pub fn write_identity(
    home: &Path,
    data_root: &Path,
    profile: &str,
    identity: &Value,
    candidates: &[Value],
) -> Result<Value> {
    let directory = user_directory(home, data_root, profile, identity)?;
    let mut record = Map::new();
    record.insert("schema".to_owned(), json!("doudou.identity.v1"));
    if let Some(id_obj) = identity.as_object() {
        for (k, v) in id_obj {
            record.insert(k.clone(), v.clone());
        }
    }
    record.insert("candidates".to_owned(), json!(candidates));
    record.insert("resolved_at".to_owned(), json!(utc_now()));

    let identity_path = directory.join("identity.json");
    atomic_write_json(&identity_path, &Value::Object(record), None)?;
    Ok(json!({
        "directory": relative_display(&directory, data_root),
        "identity": relative_display(&identity_path, data_root),
    }))
}

/// Read a persisted `identity.json` for a user if present.
pub fn read_identity(
    home: &Path,
    data_root: &Path,
    profile: &str,
    identity: &Value,
) -> Result<Option<Value>> {
    let path = user_directory(home, data_root, profile, identity)?.join("identity.json");
    read_json(&path, true)
}

/// Persist a user's video index (`videos.jsonl`) and run record (`video-run.json`).
pub fn write_video_index(
    home: &Path,
    data_root: &Path,
    profile: &str,
    identity: &Value,
    videos: &[Value],
    pages: &[Value],
) -> Result<Value> {
    let directory = user_directory(home, data_root, profile, identity)?;
    let videos_path = directory.join("videos.jsonl");
    atomic_write_jsonl(&videos_path, videos)?;
    atomic_write_json(
        &directory.join("video-run.json"),
        &json!({
            "schema": "doudou.video-index-run.v1",
            "completed_at": utc_now(),
            "count": videos.len(),
            "pages": pages,
        }),
        None,
    )?;
    Ok(json!({
        "directory": relative_display(&directory, data_root),
        "count": videos.len(),
        "videos": relative_display(&videos_path, data_root),
    }))
}

/// Read a user's persisted video index from `videos.jsonl`.
pub fn read_video_index(
    home: &Path,
    data_root: &Path,
    profile: &str,
    identity: &Value,
) -> Result<Vec<Value>> {
    let path = user_directory(home, data_root, profile, identity)?.join("videos.jsonl");
    read_jsonl(&path, true)
}

/// Persist a video's detail metadata, optional cover bytes, comments JSONL, and comment run log.
pub fn write_video(
    home: &Path,
    data_root: &Path,
    profile: &str,
    detail: &Value,
    cover: Option<&[u8]>,
    comments: &[Value],
    pages: &[Value],
) -> Result<Value> {
    let aweme_id = value_as_non_empty_string(detail.get("aweme_id"))
        .ok_or_else(|| DoudouError::new("INVALID_VIDEO", "Video detail is missing aweme_id."))?;
    let directory = video_directory(home, data_root, profile, &aweme_id)?;
    let mut metadata = detail.as_object().cloned().unwrap_or_default();
    metadata.insert("collected_at".to_owned(), json!(utc_now()));
    atomic_write_json(
        &directory.join("metadata.json"),
        &Value::Object(metadata),
        None,
    )?;
    if let Some(cover_bytes) = cover {
        atomic_write_bytes(&directory.join("cover.bin"), cover_bytes, None)?;
    }
    atomic_write_jsonl(&directory.join("comments.jsonl"), comments)?;
    atomic_write_json(
        &directory.join("comment-run.json"),
        &json!({
            "schema": "doudou.comment-run.v1",
            "completed_at": utc_now(),
            "count": comments.len(),
            "pages": pages,
        }),
        None,
    )?;
    Ok(json!({
        "directory": relative_display(&directory, data_root),
        "comments": comments.len(),
        "cover_downloaded": cover.is_some(),
    }))
}

/// Read a video's persisted `metadata.json` detail if present.
pub fn read_video_detail(
    home: &Path,
    data_root: &Path,
    profile: &str,
    aweme_id: &str,
) -> Result<Option<Value>> {
    let path = video_directory(home, data_root, profile, aweme_id)?.join("metadata.json");
    read_json(&path, true)
}
