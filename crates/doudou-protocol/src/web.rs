//! Web protocol normalizers for users, videos, `#RENDER_DATA`, and comments.

use std::collections::{BTreeMap, HashSet};

use doudou_core::{DoudouError, Result, normalize_identity};
use serde_json::{Map, Value, json};

/// Extract the first non-empty URL string from nested list/object shapes.
pub fn first_url(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Array(items) => items.iter().find_map(|item| first_url(Some(item))),
        Value::Object(map) => ["url_list", "urlList", "url", "src"]
            .iter()
            .find_map(|key| first_url(map.get(*key))),
        _ => None,
    }
}

/// Normalize a web user object if it carries at least one identifier.
pub fn identity_from(value: &Map<String, Value>) -> Option<Value> {
    let has_unique = value
        .get("unique_id")
        .or_else(|| value.get("uniqueId"))
        .is_some_and(|v| !v.is_null());
    let has_sec = value
        .get("sec_uid")
        .or_else(|| value.get("secUid"))
        .is_some_and(|v| !v.is_null());
    let has_uid = value
        .get("uid")
        .is_some_and(|v| !v.is_null() && v.as_str() != Some(""));
    if !has_unique && !has_sec && !has_uid {
        return None;
    }
    Some(normalize_identity(
        Some(&Value::Object(value.clone())),
        "web-protocol",
    ))
}

type IdentityKey = (Option<String>, Option<String>, Option<String>);

/// Walk a nested JSON value up to depth 8 and extract deduplicated, sorted identities.
pub fn identities_in(root: &Value) -> Vec<Value> {
    let mut output: BTreeMap<IdentityKey, Value> = BTreeMap::new();
    let mut pending = vec![(root, 0usize)];
    while let Some((current, depth)) = pending.pop() {
        if depth > 8 {
            continue;
        }
        match current {
            Value::Object(map) => {
                if let Some(id) = identity_from(map) {
                    let key = (
                        id.get("unique_id")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        id.get("sec_uid")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        id.get("uid").and_then(Value::as_str).map(ToOwned::to_owned),
                    );
                    output.insert(key, id);
                }
                for child in map.values() {
                    pending.push((child, depth + 1));
                }
            }
            Value::Array(items) => {
                for item in items {
                    pending.push((item, depth + 1));
                }
            }
            _ => {}
        }
    }
    let mut values: Vec<Value> = output.into_values().collect();
    values.sort_by(|a, b| {
        let key = |v: &Value| {
            (
                v.get("unique_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase(),
                v.get("sec_uid")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                v.get("uid")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            )
        };
        key(a).cmp(&key(b))
    });
    values
}

fn str_or_empty(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Normalize one item from `/aweme/v1/web/aweme/post/`.
pub fn normalize_video(raw: &Value) -> Value {
    let author = raw.get("author").or_else(|| raw.get("author_info"));
    let video = raw.get("video");
    let cover = video.and_then(|v| {
        v.get("cover")
            .or_else(|| v.get("origin_cover"))
            .or_else(|| v.get("originCover"))
    });
    json!({
        "schema": "doudou.video-index-item.v1",
        "aweme_id": str_or_empty(raw.get("aweme_id").or_else(|| raw.get("awemeId"))),
        "description": raw.get("desc").and_then(Value::as_str).unwrap_or(""),
        "create_time": raw.get("create_time").cloned().unwrap_or(Value::Null),
        "aweme_type": raw.get("aweme_type").cloned().unwrap_or(Value::Null),
        "author": normalize_identity(author, "video-list"),
        "statistics": raw.get("statistics").or_else(|| raw.get("stats")).cloned().unwrap_or_else(|| json!({})),
        "cover_url": first_url(cover),
        "duration_ms": video.and_then(|v| v.get("duration")).cloned().unwrap_or(Value::Null),
        "width": video.and_then(|v| v.get("width")).cloned().unwrap_or(Value::Null),
        "height": video.and_then(|v| v.get("height")).cloned().unwrap_or(Value::Null),
    })
}

/// Decode raw or URL-encoded JSON from a `#RENDER_DATA` script tag.
pub fn parse_render_data(text: &str) -> Result<Map<String, Value>> {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) {
        return Ok(map);
    }
    let decoded = percent_decode(text);
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&decoded) {
        return Ok(map);
    }
    Err(DoudouError::new(
        "VIDEO_DETAIL_INVALID",
        "#RENDER_DATA was not valid JSON",
    ))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Normalize a video detail payload into `doudou.video.v1`.
pub fn normalize_video_detail(detail: &Value) -> Value {
    let author = detail.get("authorInfo").or_else(|| detail.get("author"));
    let video = detail.get("video");
    let hashtags: Vec<String> = detail
        .get("textExtra")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.get("hashtagName")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(ToOwned::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    let cover = video.and_then(|v| v.get("coverUrlList").or_else(|| v.get("cover")));
    let play = video.and_then(|v| v.get("playAddr").or_else(|| v.get("play_addr")));
    json!({
        "schema": "doudou.video.v1",
        "aweme_id": str_or_empty(detail.get("awemeId").or_else(|| detail.get("aweme_id"))),
        "title": detail.get("itemTitle").or_else(|| detail.get("title")).cloned().unwrap_or(Value::Null),
        "description": detail.get("desc").and_then(Value::as_str).unwrap_or(""),
        "hashtags": hashtags,
        "create_time": detail.get("createTime").or_else(|| detail.get("create_time")).cloned().unwrap_or(Value::Null),
        "author": normalize_identity(author, "video-detail"),
        "statistics": detail.get("stats").or_else(|| detail.get("statistics")).cloned().unwrap_or_else(|| json!({})),
        "duration_ms": video.and_then(|v| v.get("duration")).cloned().unwrap_or(Value::Null),
        "width": video.and_then(|v| v.get("width")).cloned().unwrap_or(Value::Null),
        "height": video.and_then(|v| v.get("height")).cloned().unwrap_or(Value::Null),
        "cover_url": first_url(cover),
        "play_url": first_url(play),
    })
}

fn is_zero_or_empty(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty() || s == "0",
        Some(Value::Number(n)) => n.as_i64() == Some(0),
        _ => false,
    }
}

/// Normalize one comment record into `doudou.comment.v1`.
pub fn normalize_comment(raw: &Value, aweme_id: &str) -> Value {
    let cid = str_or_empty(raw.get("cid"));
    let root_id = if is_zero_or_empty(raw.get("reply_id")) {
        cid.clone()
    } else {
        str_or_empty(raw.get("reply_id"))
    };
    let direct_parent = if is_zero_or_empty(raw.get("reply_to_reply_id")) {
        None
    } else {
        Some(str_or_empty(raw.get("reply_to_reply_id")))
    };
    let user_digged = match raw.get("user_digged") {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        _ => false,
    };
    json!({
        "schema": "doudou.comment.v1",
        "aweme_id": aweme_id,
        "cid": cid,
        "text": raw.get("text").and_then(Value::as_str).unwrap_or(""),
        "create_time": raw.get("create_time").cloned().unwrap_or(Value::Null),
        "digg_count": raw.get("digg_count").cloned().unwrap_or(Value::Null),
        "user_digged": user_digged,
        "reported_reply_count": raw.get("reply_comment_total").cloned().unwrap_or(Value::Null),
        "reply_id": root_id,
        "reply_to_reply_id": direct_parent,
        "parent_resolved": direct_parent.is_none(),
        "user": normalize_identity(raw.get("user"), "comment"),
    })
}

/// Recompute `parent_resolved` flags across a collected set of comments.
pub fn resolve_comment_parents(comments: &mut [Value]) {
    let known: HashSet<String> = comments
        .iter()
        .filter_map(|c| c.get("cid").and_then(Value::as_str).map(ToOwned::to_owned))
        .collect();
    for item in comments {
        let resolved = match item.get("reply_to_reply_id").and_then(Value::as_str) {
            None => true,
            Some(parent) => known.contains(parent),
        };
        if let Some(obj) = item.as_object_mut() {
            obj.insert("parent_resolved".to_owned(), json!(resolved));
        }
    }
}
