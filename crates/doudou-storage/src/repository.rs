//! Conversation index, session-index snapshots, message JSONL merge/conflict detection, and runs.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use doudou_core::{
    DoudouError, IDENTITY_FIELDS, Result, atomic_write_json, atomic_write_jsonl, identity_matches,
    merge_identity, profile_paths, random_id, read_json, read_jsonl, safe_segment, sha256_str,
    utc_now,
};
use serde_json::{Map, Value, json};

const IDENTITY_LIST_FIELDS: &[&str] = &[
    "mentioned_users",
    "system_event.joined_users",
    "system_event.source_users",
];

const AUTHORITATIVE_INCOMING_FIELDS: &[&str] = &[
    "schema",
    "type",
    "text",
    "content",
    "system_event",
    "property_list",
    "reference_info",
    "unknown_fields",
];

fn collection_source_priority(source: Option<&str>) -> u8 {
    match source {
        Some("stranger-conversation-summary") => 0,
        Some("stranger-protocol") => 2,
        _ => 1,
    }
}

fn value_as_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn is_empty_value(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(Value::Object(map)) => map.is_empty(),
        Some(Value::Bool(_) | Value::Number(_)) => false,
    }
}

fn relative_display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Compute the deterministic folder name `<kind>__<display_name>__<conversation_id>` for a conversation.
pub fn conversation_folder(conversation: &Value) -> Result<String> {
    let kind = conversation
        .get("kind")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown");
    let display_name = conversation
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("unnamed");
    let conversation_id =
        value_as_string(conversation.get("conversation_id")).ok_or_else(|| {
            DoudouError::new(
                "INVALID_CONVERSATION",
                "Conversation record is missing a conversation_id.",
            )
        })?;
    Ok(format!(
        "{kind}__{}__{}",
        safe_segment(display_name, 64)?,
        safe_segment(&conversation_id, 64)?
    ))
}

/// Atomically persist the profile's `conversations.json` index.
pub fn write_conversation_index(
    home: &Path,
    data_root: &Path,
    profile: &str,
    conversations: &[Value],
) -> Result<Value> {
    let paths = profile_paths(home, data_root, profile)?;
    fs::create_dir_all(&paths.message).map_err(|err| {
        DoudouError::new(
            "IO_ERROR",
            format!("Failed creating {}: {err}", paths.message.display()),
        )
    })?;
    let refreshed_at = utc_now();
    let mut normalized = Vec::with_capacity(conversations.len());
    for item in conversations {
        let folder = conversation_folder(item)?;
        let mut obj = item.as_object().cloned().unwrap_or_default();
        obj.insert("folder_name".to_owned(), json!(folder));
        obj.insert("updated_at".to_owned(), json!(refreshed_at));
        normalized.push(Value::Object(obj));
    }
    let index = json!({
        "schema": "doudou.conversations-index.v2",
        "profile_id": paths.profile_id,
        "refreshed_at": refreshed_at,
        "count": normalized.len(),
        "conversations": normalized,
    });
    atomic_write_json(&paths.message.join("conversations.json"), &index, None)?;
    Ok(index)
}

/// Read the profile's `conversations.json` index if present.
pub fn read_conversation_index(
    home: &Path,
    data_root: &Path,
    profile: &str,
) -> Result<Option<Value>> {
    let path = profile_paths(home, data_root, profile)?
        .message
        .join("conversations.json");
    read_json(&path, true)
}

/// Persist a dated session-index snapshot and atomically update `session-index/latest.json`.
pub fn write_session_index_snapshot(
    home: &Path,
    data_root: &Path,
    profile: &str,
    snapshot: &Value,
) -> Result<Value> {
    let paths = profile_paths(home, data_root, profile)?;
    let completed = snapshot
        .get("completed_at")
        .and_then(Value::as_str)
        .filter(|s| s.len() >= 10)
        .ok_or_else(|| {
            DoudouError::new(
                "INVALID_SNAPSHOT",
                "Session-index snapshot is missing a valid completed_at timestamp.",
            )
        })?;
    let day = &completed[..10];
    let compact: String = completed
        .chars()
        .filter(|ch| !matches!(ch, '-' | ':' | '.'))
        .take(18)
        .collect();
    let filename = paths
        .session_index
        .join(day)
        .join(format!("{compact}__session-index.json"));
    let latest = paths.session_index.join("latest.json");
    atomic_write_json(&filename, snapshot, None)?;
    atomic_write_json(&latest, snapshot, None)?;
    Ok(json!({
        "snapshot": relative_display(&filename, data_root),
        "latest": relative_display(&latest, data_root),
    }))
}

/// Read `session-index/latest.json` if present.
pub fn read_session_index_snapshot(
    home: &Path,
    data_root: &Path,
    profile: &str,
) -> Result<Option<Value>> {
    let path = profile_paths(home, data_root, profile)?
        .session_index
        .join("latest.json");
    read_json(&path, true)
}

struct ConflictTracker {
    conflicts: Vec<Value>,
    seen: HashSet<(String, String, String, String)>,
    new_count: usize,
}

impl ConflictTracker {
    fn new(existing: Vec<Value>) -> Self {
        let seen = existing
            .iter()
            .map(|item| {
                (
                    item.get("message_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    item.get("field")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    item.get("existing_sha256")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    item.get("incoming_sha256")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                )
            })
            .collect();
        Self {
            conflicts: existing,
            seen,
            new_count: 0,
        }
    }

    fn record(&mut self, message_id: &str, field_path: &str, existing: &Value, candidate: &Value) {
        let existing_hash = sha256_str(&existing.to_string());
        let incoming_hash = sha256_str(&candidate.to_string());
        let key = (
            message_id.to_owned(),
            field_path.to_owned(),
            existing_hash.clone(),
            incoming_hash.clone(),
        );
        if !self.seen.insert(key) {
            return;
        }
        self.conflicts.push(json!({
            "schema": "doudou.conflict.v2",
            "message_id": message_id,
            "field": field_path,
            "occurred_at": utc_now(),
            "existing_sha256": existing_hash,
            "incoming_sha256": incoming_hash,
        }));
        self.new_count += 1;
    }
}

fn merge_identity_value(existing: &Map<String, Value>, candidate: &Map<String, Value>) -> Value {
    let mut merged = existing.clone();
    for (key, value) in candidate {
        if !IDENTITY_FIELDS.contains(&key.as_str())
            && (!is_empty_value(Some(value)) || !merged.contains_key(key))
        {
            merged.insert(key.clone(), value.clone());
        }
    }
    let existing_val = Value::Object(existing.clone());
    let candidate_val = Value::Object(candidate.clone());
    if let Value::Object(canonical) = merge_identity(Some(&existing_val), Some(&candidate_val)) {
        for (key, value) in canonical {
            merged.insert(key, value);
        }
    }
    Value::Object(merged)
}

fn merge_identity_list(existing: &[Value], candidate: &[Value]) -> Value {
    let mut merged: Vec<Value> = existing.to_vec();
    for item in candidate {
        let Some(item_obj) = item.as_object() else {
            if !merged.contains(item) {
                merged.push(item.clone());
            }
            continue;
        };
        if let Some(current) = merged
            .iter_mut()
            .find(|cur| cur.is_object() && identity_matches(cur, item))
        {
            if let Some(cur_obj) = current.as_object() {
                *current = merge_identity_value(cur_obj, item_obj);
            }
        } else {
            merged.push(item.clone());
        }
    }
    Value::Array(merged)
}

fn has_identity_anchor(map: &Map<String, Value>) -> bool {
    ["uid", "sec_uid", "unique_id"]
        .iter()
        .any(|key| !is_empty_value(map.get(*key)))
}

fn merge_value(
    existing: Option<&Value>,
    candidate: &Value,
    message_id: &str,
    field_path: &str,
    tracker: &mut ConflictTracker,
) -> Value {
    if is_empty_value(existing) && !is_empty_value(Some(candidate)) {
        return candidate.clone();
    }
    let Some(existing_val) = existing else {
        return candidate.clone();
    };
    if field_path.ends_with(".enrichment.updated_at") {
        return if is_empty_value(Some(existing_val)) {
            candidate.clone()
        } else {
            existing_val.clone()
        };
    }
    if let (Value::Array(ex_arr), Value::Array(cand_arr)) = (existing_val, candidate)
        && IDENTITY_LIST_FIELDS.contains(&field_path)
    {
        return merge_identity_list(ex_arr, cand_arr);
    }
    if let (Value::Object(ex_obj), Value::Object(cand_obj)) = (existing_val, candidate) {
        if has_identity_anchor(ex_obj) || has_identity_anchor(cand_obj) {
            return merge_identity_value(ex_obj, cand_obj);
        }
        let mut merged = ex_obj.clone();
        for (key, value) in cand_obj {
            let child_path = if field_path.is_empty() {
                key.clone()
            } else {
                format!("{field_path}.{key}")
            };
            let next = merge_value(merged.get(key), value, message_id, &child_path, tracker);
            merged.insert(key.clone(), next);
        }
        return Value::Object(merged);
    }
    if !is_empty_value(Some(candidate))
        && existing_val != candidate
        && !matches!(field_path, "collected_at" | "run_id")
    {
        let root_field = field_path.split('.').next().unwrap_or(field_path);
        if AUTHORITATIVE_INCOMING_FIELDS.contains(&root_field) {
            return candidate.clone();
        }
        tracker.record(message_id, field_path, existing_val, candidate);
    }
    existing_val.clone()
}

/// Merge incoming messages into a conversation's `messages.jsonl`, deduplicating and logging conflicts.
pub fn merge_messages(
    home: &Path,
    data_root: &Path,
    profile: &str,
    conversation: &Value,
    incoming: &[Value],
) -> Result<Value> {
    let paths = profile_paths(home, data_root, profile)?;
    let folder = conversation_folder(conversation)?;
    let directory = paths.conversations.join(&folder);
    fs::create_dir_all(&directory).map_err(|err| {
        DoudouError::new(
            "IO_ERROR",
            format!("Failed creating {}: {err}", directory.display()),
        )
    })?;

    let mut metadata = Map::new();
    metadata.insert("schema".to_owned(), json!("doudou.conversation.v2"));
    if let Some(conv_obj) = conversation.as_object() {
        for (k, v) in conv_obj {
            metadata.insert(k.clone(), v.clone());
        }
    }
    metadata.insert("profile_id".to_owned(), json!(paths.profile_id));
    metadata.insert("folder_name".to_owned(), json!(folder));
    metadata.insert("updated_at".to_owned(), json!(utc_now()));
    atomic_write_json(
        &directory.join("conversation.json"),
        &Value::Object(metadata),
        None,
    )?;

    let existing_records = read_jsonl(&directory.join("messages.jsonl"), true)?;
    let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
    for item in existing_records {
        if let Some(mid) = value_as_string(item.get("message_id")) {
            by_id.insert(mid, item);
        }
    }
    let existing_conflicts = read_jsonl(&directory.join("conflicts.jsonl"), true)?;
    let mut tracker = ConflictTracker::new(existing_conflicts);

    let mut added: usize = 0;
    let mut duplicates: usize = 0;
    for item in incoming {
        let identity = value_as_string(item.get("message_id")).ok_or_else(|| {
            DoudouError::new("INVALID_MESSAGE", "Incoming message is missing message_id.")
        })?;
        let Some(previous) = by_id.get(&identity).cloned() else {
            by_id.insert(identity, item.clone());
            added += 1;
            continue;
        };
        duplicates += 1;
        let mut merged = previous.as_object().cloned().unwrap_or_default();
        let previous_priority =
            collection_source_priority(previous.get("collection_source").and_then(Value::as_str));
        let incoming_priority =
            collection_source_priority(item.get("collection_source").and_then(Value::as_str));

        if let Some(item_obj) = item.as_object() {
            for (key, value) in item_obj {
                if key == "collection_source" && incoming_priority > previous_priority {
                    merged.insert(key.clone(), value.clone());
                    continue;
                }
                let is_protected = AUTHORITATIVE_INCOMING_FIELDS.contains(&key.as_str())
                    || matches!(key.as_str(), "collection_source" | "images");
                if incoming_priority < previous_priority
                    && is_protected
                    && !is_empty_value(merged.get(key))
                {
                    continue;
                }
                if key == "images" {
                    merged.insert(key.clone(), value.clone());
                    continue;
                }
                let next = merge_value(merged.get(key), value, &identity, key, &mut tracker);
                merged.insert(key.clone(), next);
            }
        }
        by_id.insert(identity, Value::Object(merged));
    }

    let mut records: Vec<Value> = by_id.into_values().collect();
    records.sort_by(|left, right| {
        let sort_key = |item: &Value| {
            let ts = item
                .get("create_time_ms")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let mid_str = value_as_string(item.get("message_id")).unwrap_or_default();
            let mid_num = mid_str.parse::<i128>().unwrap_or(0);
            (ts, mid_num, mid_str)
        };
        sort_key(left).cmp(&sort_key(right))
    });

    let messages_file = directory.join("messages.jsonl");
    atomic_write_jsonl(&messages_file, &records)?;
    atomic_write_jsonl(&directory.join("conflicts.jsonl"), &tracker.conflicts)?;

    Ok(json!({
        "directory": directory.display().to_string(),
        "messages_file": messages_file.display().to_string(),
        "total": records.len(),
        "added": added,
        "duplicates": duplicates,
        "conflicts": tracker.new_count,
        "conflicts_total": tracker.conflicts.len(),
    }))
}

/// Read all persisted messages for a conversation from `messages.jsonl`.
pub fn read_messages(
    home: &Path,
    data_root: &Path,
    profile: &str,
    conversation: &Value,
) -> Result<Vec<Value>> {
    let directory = profile_paths(home, data_root, profile)?
        .conversations
        .join(conversation_folder(conversation)?);
    read_jsonl(&directory.join("messages.jsonl"), true)
}

/// Read recorded field merge conflicts for a conversation from `conflicts.jsonl`.
pub fn read_conflicts(
    home: &Path,
    data_root: &Path,
    profile: &str,
    conversation: &Value,
) -> Result<Vec<Value>> {
    let directory = profile_paths(home, data_root, profile)?
        .conversations
        .join(conversation_folder(conversation)?);
    read_jsonl(&directory.join("conflicts.jsonl"), true)
}

/// Tally unknown protobuf fields across a batch of messages into a deterministic inventory map.
pub fn unknown_field_inventory(messages: &[Value]) -> BTreeMap<String, usize> {
    let mut inventory = BTreeMap::new();
    for message in messages {
        let Some(fields) = message.get("unknown_fields").and_then(Value::as_array) else {
            continue;
        };
        for field in fields {
            let Some(number) = field.get("field").and_then(Value::as_u64) else {
                continue;
            };
            let Some(wire) = field.get("wire").and_then(Value::as_u64) else {
                continue;
            };
            let key = format!("field_{number}_wire_{wire}");
            *inventory.entry(key).or_insert(0) += 1;
        }
    }
    inventory
}

/// Atomically persist a collection run summary under `message/profiles/<profile>/runs/`.
pub fn write_run(home: &Path, data_root: &Path, profile: &str, run: &Value) -> Result<Value> {
    let paths = profile_paths(home, data_root, profile)?;
    let run_id = run
        .get("run_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map_or_else(|| random_id("run"), ToOwned::to_owned);
    let mut record = run.as_object().cloned().unwrap_or_default();
    record.insert("run_id".to_owned(), json!(run_id));
    record
        .entry("completed_at".to_owned())
        .or_insert_with(|| json!(utc_now()));
    let file = paths
        .runs
        .join(format!("{}.json", safe_segment(&run_id, 96)?));
    atomic_write_json(&file, &Value::Object(record), None)?;
    Ok(json!({
        "run_id": run_id,
        "path": relative_display(&file, data_root),
    }))
}

/// Read one persisted run summary by `run_id`.
pub fn read_run(
    home: &Path,
    data_root: &Path,
    profile: &str,
    run_id: &str,
) -> Result<Option<Value>> {
    let file = profile_paths(home, data_root, profile)?
        .runs
        .join(format!("{}.json", safe_segment(run_id, 96)?));
    read_json(&file, true)
}

/// List all persisted run summaries for a profile, ordered by `completed_at` ascending.
pub fn list_runs(home: &Path, data_root: &Path, profile: &str) -> Result<Vec<Value>> {
    let dir = profile_paths(home, data_root, profile)?.runs;
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Some(value) = read_json(&path, true)? {
            runs.push(value);
        }
    }
    runs.sort_by(|left, right| {
        let key = |v: &Value| {
            (
                v.get("completed_at")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                v.get("run_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            )
        };
        key(left).cmp(&key(right))
    });
    Ok(runs)
}
