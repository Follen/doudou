//! Douyin IM Protobuf command builders and response parsers (established + stranger inboxes).

use std::collections::{BTreeMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use doudou_core::{DoudouError, Result, conversation_kind, normalize_identity};
use serde_json::{Map, Value, json};

use crate::protobuf::{
    Field, bytes_field, decode_fields, encode_fields, get_field, get_fields, replace_field,
    string_field, text_value, varint_field,
};

const MAX_NESTED_PROTOBUF_BYTES: usize = 10 * 1024 * 1024;

/// Map stranger-inbox command ID (`1001` or `1002`) to its body field number (`1000` or `1001`).
pub fn stranger_body_field(command: u32) -> Result<u32> {
    match command {
        1001 => Ok(1000),
        1002 => Ok(1001),
        _ => Err(DoudouError::new(
            "UNSUPPORTED_COMMAND",
            format!("unsupported stranger command: {command}"),
        )),
    }
}

fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
}

/// Build an IM command envelope using the stranger command/body mapping.
pub fn stranger_request(
    init_request: &[u8],
    command: u32,
    body: &[u8],
    sequence: Option<u64>,
) -> Result<Vec<u8>> {
    let body_field = stranger_body_field(command)?;
    let outer = decode_fields(init_request)?;
    let outer = replace_field(outer, 1, varint_field(1, u64::from(command)));
    let outer = replace_field(
        outer,
        2,
        varint_field(2, sequence.unwrap_or_else(now_micros)),
    );
    let inner = encode_fields(&[bytes_field(body_field, body.to_vec())]);
    let outer = replace_field(outer, 8, bytes_field(8, inner));
    Ok(encode_fields(&outer))
}

/// Encode command 1001 (`get_stranger_conversation_list`) body.
pub fn stranger_conversations_request(cursor: u64, count: u64, show_total_unread: bool) -> Vec<u8> {
    encode_fields(&[
        varint_field(1, cursor),
        varint_field(2, count.max(1)),
        varint_field(3, u64::from(show_total_unread)),
    ])
}

/// Encode command 1002 (`get_stranger_messages`) body.
pub fn stranger_messages_request(conversation_short_id: u64, reset_unread_count: bool) -> Vec<u8> {
    encode_fields(&[
        varint_field(1, conversation_short_id),
        varint_field(2, u64::from(reset_unread_count)),
    ])
}

/// Parsed outer IM response envelope.
#[derive(Debug, Clone)]
pub struct ImResponse {
    pub status: i64,
    pub status_text: String,
    pub body: Option<Vec<u8>>,
    pub gateway: Option<Value>,
}

/// Parse a stranger-inbox response envelope using its mapped body field.
pub fn stranger_response_body(data: &[u8], command: u32) -> Result<ImResponse> {
    let field = stranger_body_field(command)?;
    response_body(data, field)
}

/// Build a standard IM command request envelope from a captured `init_request`.
pub fn build_command_request(
    init_request: &[u8],
    command: u32,
    body: &[u8],
    sequence: Option<u64>,
) -> Result<Vec<u8>> {
    let outer = decode_fields(init_request)?;
    let outer = replace_field(outer, 1, varint_field(1, u64::from(command)));
    let outer = replace_field(
        outer,
        2,
        varint_field(2, sequence.unwrap_or_else(now_micros)),
    );
    let inner = encode_fields(&[bytes_field(command, body.to_vec())]);
    let outer = replace_field(outer, 8, bytes_field(8, inner));
    Ok(encode_fields(&outer))
}

/// Parse an outer IM Protobuf (or JSON gateway) response for `command`.
pub fn response_body(data: &[u8], command: u32) -> Result<ImResponse> {
    let trimmed = trim_leading_ascii_whitespace(data);
    if trimmed.starts_with(b"{") || trimmed.starts_with(b"[") {
        let gateway = serde_json::from_slice::<Value>(data)
            .unwrap_or_else(|_| json!({ "invalid_json": true }));
        return Ok(ImResponse {
            status: -1,
            status_text: "json gateway".to_owned(),
            body: None,
            gateway: Some(gateway),
        });
    }
    let outer = decode_fields(data)?;
    let status = get_field(&outer, 3, Some(0))
        .and_then(Field::as_varint)
        .map_or(-1, |v| v as i64);
    let status_text = text_value(get_field(&outer, 4, Some(2))).unwrap_or_default();
    let Some(payload) = get_field(&outer, 6, Some(2)).and_then(Field::as_bytes) else {
        return Ok(ImResponse {
            status,
            status_text,
            body: None,
            gateway: None,
        });
    };
    let wrapper = decode_fields(payload)?;
    let body = get_field(&wrapper, command, Some(2))
        .and_then(Field::as_bytes)
        .map(ToOwned::to_owned);
    Ok(ImResponse {
        status,
        status_text,
        body,
        gateway: None,
    })
}

fn trim_leading_ascii_whitespace(data: &[u8]) -> &[u8] {
    let idx = data
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(data.len());
    &data[idx..]
}

/// Parse repeated `map<string, string>` entries at `field_number`.
pub fn parse_map_entries(fields: &[Field], field_number: u32) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for item in get_fields(fields, field_number, Some(2)) {
        let Some(bytes) = item.as_bytes() else {
            continue;
        };
        let Ok(entry) = decode_fields(bytes) else {
            continue;
        };
        if let (Some(key), Some(val)) = (
            text_value(get_field(&entry, 1, Some(2))),
            text_value(get_field(&entry, 2, Some(2))),
        ) {
            out.insert(key, val);
        }
    }
    out
}

/// Format an unknown Protobuf field for diagnostic inventory without dumping unbounded binary.
pub fn structured_field(item: &Field) -> Value {
    if item.wire == 0 {
        let val = item.as_varint().unwrap_or(0);
        return json!({
            "field": item.number,
            "wire": 0,
            "value": val.to_string(),
        });
    }
    let bytes = item.as_bytes().unwrap_or(&[]);
    let mut map = Map::new();
    map.insert("field".to_owned(), json!(item.number));
    map.insert("wire".to_owned(), json!(item.wire));
    if let Some(text) = text_value(Some(item)) {
        let truncated: String = text.chars().take(500).collect();
        map.insert("text".to_owned(), json!(truncated));
    } else {
        let prefix = &bytes[..bytes.len().min(16)];
        map.insert("prefix_hex".to_owned(), json!(hex::encode(prefix)));
    }
    map.insert("bytes".to_owned(), json!(bytes.len()));
    Value::Object(map)
}

fn stranger_record(fields: &[Field]) -> Option<Value> {
    let short = get_field(fields, 1, Some(0))?.as_varint()?;
    let identity = text_value(get_field(fields, 4, Some(2)))?;
    let mut participants: Vec<Value> = Vec::new();
    for item in get_fields(fields, 5, Some(2)) {
        if let Some(bytes) = item.as_bytes() {
            let wrapped = encode_fields(&[bytes_field(1, bytes.to_vec())]);
            let page = parse_participants_page(&wrapped);
            if let Some(arr) = page.get("participants").and_then(Value::as_array) {
                participants.extend(arr.iter().cloned());
            }
        }
    }
    let last_message = get_field(fields, 3, Some(2))
        .and_then(Field::as_bytes)
        .and_then(|b| normalize_message(b, &identity));
    let unread = get_field(fields, 2, Some(0))
        .and_then(Field::as_varint)
        .unwrap_or(0);
    let mut names: HashSet<String> = HashSet::new();
    for item in &participants {
        let candidate = item
            .get("display_name")
            .and_then(Value::as_str)
            .or_else(|| item.get("alias").and_then(Value::as_str))
            .unwrap_or("")
            .trim();
        if !candidate.is_empty() {
            names.insert(candidate.to_owned());
        }
    }
    let display_name = if names.len() == 1 {
        names.into_iter().next().unwrap_or_else(|| identity.clone())
    } else {
        identity.clone()
    };
    Some(json!({
        "conversation_id": identity,
        "conversation_short_id": short.to_string(),
        "conversation_type": 1,
        "kind": "direct",
        "inbox": "request",
        "display_name": display_name,
        "unread_count": unread,
        "participants": {
            "participants": participants,
            "has_more": false,
            "cursor": "0",
        },
        "last_message": last_message,
        "ext": parse_map_entries(fields, 7),
    }))
}

/// Normalize command 1001 stranger conversations response body.
pub fn parse_stranger_conversations(body: &[u8]) -> Value {
    let Ok(fields) = decode_fields(body) else {
        return json!({
            "conversations": [],
            "next_cursor": "0",
            "has_more": false,
            "total_unread": 0,
        });
    };
    let mut conversations = Vec::new();
    for item in get_fields(&fields, 4, Some(2)) {
        if let Some(bytes) = item.as_bytes()
            && let Ok(sub) = decode_fields(bytes)
            && let Some(rec) = stranger_record(&sub)
        {
            conversations.push(rec);
        }
    }
    let next_cursor = get_field(&fields, 1, Some(0))
        .and_then(Field::as_varint)
        .map_or_else(|| "0".to_owned(), |v| v.to_string());
    let has_more = get_field(&fields, 2, Some(0))
        .and_then(Field::as_varint)
        .is_some_and(|v| v != 0);
    let total_unread = get_field(&fields, 3, Some(0))
        .and_then(Field::as_varint)
        .unwrap_or(0);
    json!({
        "conversations": conversations,
        "next_cursor": next_cursor,
        "has_more": has_more,
        "total_unread": total_unread,
    })
}

/// Extract normalized message records from a command 1002 stranger messages response.
pub fn parse_stranger_messages(body: &[u8], conversation_id: Option<&str>) -> Vec<Value> {
    let Ok(fields) = decode_fields(body) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for field_num in [3u32, 1, 2] {
        for item in get_fields(&fields, field_num, Some(2)) {
            let Some(bytes) = item.as_bytes() else {
                continue;
            };
            if let Some(cid) = conversation_id
                && let Some(msg) = normalize_message(bytes, cid)
            {
                out.push(msg);
            } else {
                out.extend(parse_stranger_messages(bytes, conversation_id));
            }
        }
    }
    out
}

/// Walk an IM init response payload and extract all established direct/group conversations.
pub fn find_conversations(data: &[u8]) -> Vec<Value> {
    let mut found: BTreeMap<String, Value> = BTreeMap::new();
    let mut pending: Vec<(&[u8], usize)> = vec![(data, 0)];
    while let Some((value, depth)) = pending.pop() {
        if depth > 10 || value.is_empty() {
            continue;
        }
        let Ok(fields) = decode_fields(value) else {
            continue;
        };
        let identity = text_value(get_field(&fields, 1, Some(2)));
        let short = get_field(&fields, 2, Some(0)).and_then(Field::as_varint);
        let kind = get_field(&fields, 3, Some(0)).and_then(Field::as_varint);
        let ticket = text_value(get_field(&fields, 4, Some(2)));
        if let (Some(id), Some(short_id), Some(kind_val), Some(_)) = (identity, short, kind, ticket)
            && matches!(kind_val, 1 | 2)
        {
            let mut display_name: Option<String> = None;
            if let Some(core_bytes) = get_field(&fields, 50, Some(2)).and_then(Field::as_bytes)
                && let Ok(core_fields) = decode_fields(core_bytes)
                && text_value(get_field(&core_fields, 1, Some(2))).as_deref() == Some(id.as_str())
            {
                display_name = text_value(get_field(&core_fields, 5, Some(2)));
            }
            let participants_bytes = get_field(&fields, 6, Some(2))
                .and_then(Field::as_bytes)
                .unwrap_or(&[]);
            found.insert(
                id.clone(),
                json!({
                    "conversation_id": id,
                    "conversation_short_id": short_id.to_string(),
                    "conversation_type": kind_val,
                    "kind": conversation_kind(kind_val as i64),
                    "inbox": "established",
                    "display_name": display_name.unwrap_or_else(|| id.clone()),
                    "participants": parse_participants_page(participants_bytes),
                }),
            );
        }
        for item in fields {
            if item.wire == 2
                && let Some(bytes) = item.as_bytes()
                && bytes.len() <= MAX_NESTED_PROTOBUF_BYTES
            {
                // Owned recursion via helper to avoid borrowing local `fields`.
                find_conversations_nested(bytes, depth + 1, &mut found);
            }
        }
    }
    found.into_values().collect()
}

fn find_conversations_nested(data: &[u8], depth: usize, found: &mut BTreeMap<String, Value>) {
    if depth > 10 || data.is_empty() {
        return;
    }
    let Ok(fields) = decode_fields(data) else {
        return;
    };
    let identity = text_value(get_field(&fields, 1, Some(2)));
    let short = get_field(&fields, 2, Some(0)).and_then(Field::as_varint);
    let kind = get_field(&fields, 3, Some(0)).and_then(Field::as_varint);
    let ticket = text_value(get_field(&fields, 4, Some(2)));
    if let (Some(id), Some(short_id), Some(kind_val), Some(_)) = (identity, short, kind, ticket)
        && matches!(kind_val, 1 | 2)
    {
        let mut display_name: Option<String> = None;
        if let Some(core_bytes) = get_field(&fields, 50, Some(2)).and_then(Field::as_bytes)
            && let Ok(core_fields) = decode_fields(core_bytes)
            && text_value(get_field(&core_fields, 1, Some(2))).as_deref() == Some(id.as_str())
        {
            display_name = text_value(get_field(&core_fields, 5, Some(2)));
        }
        let participants_bytes = get_field(&fields, 6, Some(2))
            .and_then(Field::as_bytes)
            .unwrap_or(&[]);
        found.insert(
            id.clone(),
            json!({
                "conversation_id": id,
                "conversation_short_id": short_id.to_string(),
                "conversation_type": kind_val,
                "kind": conversation_kind(kind_val as i64),
                "inbox": "established",
                "display_name": display_name.unwrap_or_else(|| id.clone()),
                "participants": parse_participants_page(participants_bytes),
            }),
        );
    }
    for item in fields {
        if item.wire == 2
            && let Some(bytes) = item.as_bytes()
            && bytes.len() <= MAX_NESTED_PROTOBUF_BYTES
        {
            find_conversations_nested(bytes, depth + 1, found);
        }
    }
}

/// Parse one page of conversation participants.
pub fn parse_participants_page(data: &[u8]) -> Value {
    if data.is_empty() {
        return json!({"participants": [], "has_more": false, "cursor": "0"});
    }
    let Ok(fields) = decode_fields(data) else {
        return json!({"participants": [], "has_more": false, "cursor": "0"});
    };
    let mut participants = Vec::new();
    for item in get_fields(&fields, 1, Some(2)) {
        let Some(bytes) = item.as_bytes() else {
            continue;
        };
        let Ok(record) = decode_fields(bytes) else {
            continue;
        };
        let Some(uid) = get_field(&record, 1, Some(0)).and_then(Field::as_varint) else {
            continue;
        };
        let alias = text_value(get_field(&record, 4, Some(2))).unwrap_or_default();
        let sec_uid = text_value(get_field(&record, 5, Some(2))).unwrap_or_default();
        let mut norm = normalize_identity(
            Some(&json!({
                "uid": uid.to_string(),
                "sec_uid": sec_uid,
                "remark_name": alias,
            })),
            "im-participant",
        );
        if let Some(obj) = norm.as_object_mut() {
            obj.insert(
                "sort_order".to_owned(),
                get_field(&record, 2, Some(0))
                    .and_then(Field::as_varint)
                    .map_or(Value::Null, |v| json!(v.to_string())),
            );
            obj.insert(
                "role".to_owned(),
                json!(
                    get_field(&record, 3, Some(0))
                        .and_then(Field::as_varint)
                        .unwrap_or(0)
                ),
            );
            obj.insert("alias".to_owned(), json!(alias));
            obj.insert("ext".to_owned(), json!(parse_map_entries(&record, 8)));
        }
        participants.push(norm);
    }
    let has_more = get_field(&fields, 2, Some(0))
        .and_then(Field::as_varint)
        .is_some_and(|v| v != 0);
    let cursor = get_field(&fields, 3, Some(0))
        .and_then(Field::as_varint)
        .map_or_else(|| "0".to_owned(), |v| v.to_string());
    json!({
        "participants": participants,
        "has_more": has_more,
        "cursor": cursor,
    })
}

/// Encode command 605 (`participants_by_conversation`) request body.
pub fn participants_request(
    conversation_id: &str,
    conversation_short_id: u64,
    conversation_type: u64,
    cursor: u64,
    limit: u64,
) -> Vec<u8> {
    encode_fields(&[
        string_field(1, conversation_id),
        varint_field(2, conversation_short_id),
        varint_field(3, conversation_type),
        varint_field(4, cursor),
        varint_field(5, limit),
    ])
}

/// Normalize system event messages (types `1001` and `50001`).
pub fn normalize_system_event(content: &Value, message_type: u64) -> Option<Value> {
    if !matches!(message_type, 1001 | 50001) {
        return None;
    }
    let obj = content.as_object();
    let awe_type = obj.and_then(|m| m.get("aweType")).and_then(Value::as_i64);
    if awe_type != Some(100_140) {
        let shape = match obj {
            Some(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                json!(keys)
            }
            None => json!(json_type_name(content)),
        };
        return Some(json!({
            "kind": "unknown",
            "protocol": {
                "message_type": message_type,
                "awe_type": awe_type,
            },
            "content_shape": shape,
        }));
    }
    let map = obj?;
    let users = |key: &str| -> Vec<Value> {
        map.get(key)
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .map(|item| normalize_identity(Some(item), &format!("member-joined-{key}")))
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(json!({
        "kind": "member_joined",
        "protocol": {"message_type": 1001, "awe_type": 100_140},
        "join_method": "profile_homepage",
        "history_visible": true,
        "source_users": users("active_users"),
        "joined_users": users("passive_users"),
        "locale_resources": map.get("locale_resources").cloned().unwrap_or_else(|| json!([])),
        "template_actions": map.get("template").cloned().unwrap_or_else(|| json!([])),
        "content_ext": map.get("content_ext").cloned().unwrap_or(Value::Null),
    }))
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// Parse field 18 (`reference_info`) of an IM message.
pub fn parse_reference_info(item: Option<&Field>) -> Option<Value> {
    let field = item?;
    if field.wire != 2 {
        return None;
    }
    let fields = decode_fields(field.as_bytes()?).ok()?;
    let raw = text_value(get_field(&fields, 2, Some(2)));
    let hint = match &raw {
        Some(text) => {
            serde_json::from_str::<Value>(text).unwrap_or_else(|_| Value::String(text.clone()))
        }
        None => Value::Null,
    };
    let unknown: Vec<Value> = fields
        .iter()
        .filter(|f| !(1..=7).contains(&f.number))
        .map(structured_field)
        .collect();
    Some(json!({
        "referenced_message_id": get_field(&fields, 1, Some(0)).and_then(Field::as_varint).map(|v| v.to_string()),
        "hint": hint,
        "hint_raw": raw,
        "ref_message_type": get_field(&fields, 3, Some(0)).and_then(Field::as_varint),
        "referenced_message_status": get_field(&fields, 4, Some(0)).and_then(Field::as_varint),
        "root_message_id": get_field(&fields, 5, Some(0)).and_then(Field::as_varint).map(|v| v.to_string()),
        "root_message_conv_index": get_field(&fields, 6, Some(0)).and_then(Field::as_varint).map(|v| v.to_string()),
        "ext": parse_map_entries(&fields, 7),
        "unknown_fields": unknown,
    }))
}

/// Parse field 15 (`property_list`, e.g., chat likes/reactions) of an IM message.
pub fn parse_property_list(fields: &[Field]) -> Map<String, Value> {
    let mut output = Map::new();
    for item in get_fields(fields, 15, Some(2)) {
        let Some(bytes) = item.as_bytes() else {
            continue;
        };
        let Ok(entry) = decode_fields(bytes) else {
            continue;
        };
        let Some(key) = text_value(get_field(&entry, 1, Some(2))) else {
            continue;
        };
        let Some(val_bytes) = get_field(&entry, 2, Some(2)).and_then(Field::as_bytes) else {
            continue;
        };
        let Ok(list_fields) = decode_fields(val_bytes) else {
            continue;
        };
        let mut items = Vec::new();
        for child in get_fields(&list_fields, 1, Some(2)) {
            let Some(child_bytes) = child.as_bytes() else {
                continue;
            };
            let Ok(f2) = decode_fields(child_bytes) else {
                continue;
            };
            let uid = get_field(&f2, 1, Some(0))
                .and_then(Field::as_varint)
                .map(|v| v.to_string());
            let sec_uid = text_value(get_field(&f2, 2, Some(2)));
            let mut identity = normalize_identity(
                Some(&json!({"uid": uid, "sec_uid": sec_uid})),
                &format!("message-property-{key}"),
            );
            if let Some(obj) = identity.as_object_mut() {
                obj.insert(
                    "create_time".to_owned(),
                    get_field(&f2, 3, Some(0))
                        .and_then(Field::as_varint)
                        .map_or(Value::Null, |v| json!(v.to_string())),
                );
                obj.insert(
                    "idempotent_id".to_owned(),
                    json!(text_value(get_field(&f2, 4, Some(2)))),
                );
                obj.insert(
                    "value".to_owned(),
                    json!(text_value(get_field(&f2, 5, Some(2)))),
                );
            }
            items.push(identity);
        }
        output.insert(key, json!({ "items": items }));
    }
    output
}

/// Check whether `property_list` contains an entry attributed to `uid`.
pub fn property_has_user(property_list: Option<&Value>, uid: &str) -> bool {
    let Some(map) = property_list.and_then(Value::as_object) else {
        return false;
    };
    map.values().any(|group| {
        group
            .get("items")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.get("uid")
                        .and_then(|u| {
                            u.as_str()
                                .map(ToOwned::to_owned)
                                .or_else(|| u.as_i64().map(|n| n.to_string()))
                        })
                        .as_deref()
                        == Some(uid)
                })
            })
    })
}

/// Decode one Protobuf IM message belonging to `expected_conversation` into `doudou.message.v2`.
pub fn normalize_message(data: &[u8], expected_conversation: &str) -> Option<Value> {
    let fields = decode_fields(data).ok()?;
    let conversation_id = text_value(get_field(&fields, 1, Some(2)))?;
    if conversation_id != expected_conversation {
        return None;
    }
    let message_id = get_field(&fields, 3, Some(0))?.as_varint()?;
    let message_type = get_field(&fields, 6, Some(0))?.as_varint()?;
    let create_time = get_field(&fields, 10, Some(0))?.as_varint()?;
    let content_text = text_value(get_field(&fields, 8, Some(2)))?;

    let content: Value =
        serde_json::from_str(&content_text).unwrap_or_else(|_| Value::String(content_text.clone()));
    let reference = parse_reference_info(get_field(&fields, 18, Some(2)));
    let kind = match message_type {
        1001 | 50001 => "system",
        27 => "image",
        7 => "text",
        _ => "unsupported",
    };
    let text: Value = if let Some(obj) = content.as_object() {
        obj.get("text").cloned().unwrap_or(Value::Null)
    } else if kind == "text" && content.is_string() {
        content.clone()
    } else {
        Value::Null
    };
    let sender = normalize_identity(
        Some(&json!({
            "uid": get_field(&fields, 7, Some(0)).and_then(Field::as_varint).map(|v| v.to_string()),
            "sec_uid": text_value(get_field(&fields, 14, Some(2))),
        })),
        "im-message",
    );
    let reply_to = reference
        .as_ref()
        .and_then(|r| r.get("referenced_message_id"))
        .cloned()
        .unwrap_or(Value::Null);
    let unknown: Vec<Value> = fields
        .iter()
        .filter(|f| !((1..=15).contains(&f.number) || matches!(f.number, 17 | 18)))
        .map(structured_field)
        .collect();

    Some(json!({
        "schema": "doudou.message.v2",
        "message_id": message_id.to_string(),
        "conversation_id": conversation_id,
        "conversation_type": get_field(&fields, 2, Some(0)).and_then(Field::as_varint),
        "conversation_short_id": get_field(&fields, 5, Some(0)).and_then(Field::as_varint).map(|v| v.to_string()),
        "message_type": message_type,
        "type": kind,
        "sender": sender,
        "create_time_ms": create_time,
        "text": text,
        "content": content,
        "system_event": normalize_system_event(&content, message_type),
        "reference_info": reference,
        "reply_to_message_id": reply_to,
        "ext": parse_map_entries(&fields, 9),
        "property_list": parse_property_list(&fields),
        "unknown_fields": unknown,
        "images": [],
    }))
}

/// Recursively extract and sort all messages matching `conversation_id` from a command 301 response body.
pub fn extract_messages(body: &[u8], conversation_id: &str) -> Vec<Value> {
    fn visit(value: &[u8], depth: usize, conversation_id: &str, out: &mut BTreeMap<String, Value>) {
        if depth > 8 || value.is_empty() {
            return;
        }
        if let Some(msg) = normalize_message(value, conversation_id)
            && let Some(mid) = msg.get("message_id").and_then(Value::as_str)
        {
            out.insert(mid.to_owned(), msg);
        }
        let Ok(fields) = decode_fields(value) else {
            return;
        };
        for item in fields {
            if item.wire == 2
                && let Some(bytes) = item.as_bytes()
                && bytes.len() <= MAX_NESTED_PROTOBUF_BYTES
            {
                visit(bytes, depth + 1, conversation_id, out);
            }
        }
    }

    let mut out = BTreeMap::new();
    visit(body, 0, conversation_id, &mut out);
    let mut values: Vec<Value> = out.into_values().collect();
    values.sort_by_key(|item| {
        let ts = item
            .get("create_time_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mid = item
            .get("message_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u128>().ok())
            .unwrap_or(0);
        (ts, mid)
    });
    values
}

/// Encode command 301 (`get_by_conversation`) request body.
pub fn messages_request(
    conversation_id: &str,
    conversation_type: u64,
    conversation_short_id: u64,
    anchor_index: u64,
    limit: u64,
    direction: u64,
) -> Vec<u8> {
    encode_fields(&[
        string_field(1, conversation_id),
        varint_field(2, conversation_type),
        varint_field(3, conversation_short_id),
        varint_field(4, direction),
        varint_field(5, anchor_index),
        varint_field(6, limit),
    ])
}

/// Parse an outgoing `/v1/message/send` Protobuf request envelope for receipt verification.
pub fn parse_send_request(data: &[u8]) -> Result<Value> {
    let outer = decode_fields(data)?;
    let command_wrapper = get_field(&outer, 8, Some(2))
        .and_then(Field::as_bytes)
        .ok_or_else(|| {
            DoudouError::new(
                "INVALID_PROTOBUF",
                "send request command wrapper is missing",
            )
        })?;
    let command_fields = decode_fields(command_wrapper)?;
    let body = get_field(&command_fields, 100, Some(2))
        .and_then(Field::as_bytes)
        .ok_or_else(|| DoudouError::new("INVALID_PROTOBUF", "send request body is missing"))?;
    let fields = decode_fields(body)?;
    let content_text = text_value(get_field(&fields, 4, Some(2)));
    let content = match content_text {
        Some(text) => serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text)),
        None => Value::Null,
    };
    let reference_id = get_field(&fields, 11, Some(2))
        .and_then(Field::as_bytes)
        .and_then(|b| decode_fields(b).ok())
        .and_then(|rf| get_field(&rf, 1, Some(0)).and_then(Field::as_varint))
        .map(|v| v.to_string());
    let mentioned: Vec<String> = get_fields(&fields, 9, Some(0))
        .into_iter()
        .filter_map(Field::as_varint)
        .map(|v| v.to_string())
        .collect();
    Ok(json!({
        "conversation_id": text_value(get_field(&fields, 1, Some(2))),
        "conversation_type": get_field(&fields, 2, Some(0)).and_then(Field::as_varint),
        "conversation_short_id": get_field(&fields, 3, Some(0)).and_then(Field::as_varint).map(|v| v.to_string()),
        "content": content,
        "ext": parse_map_entries(&fields, 5),
        "message_type": get_field(&fields, 6, Some(0)).and_then(Field::as_varint),
        "client_message_id": text_value(get_field(&fields, 8, Some(2))),
        "mentioned_users": mentioned,
        "referenced_message_id": reference_id,
    }))
}

/// Parse a `/v1/message/send` Protobuf (or JSON gateway) response envelope.
pub fn parse_send_response(data: &[u8]) -> Result<Value> {
    let resp = response_body(data, 100)?;
    if let Some(gateway) = resp.gateway {
        return Ok(json!({
            "protobuf_status": resp.status,
            "status_text": resp.status_text,
            "gateway": gateway,
        }));
    }
    let body_fields = match resp.body {
        Some(bytes) => decode_fields(&bytes)?,
        None => Vec::new(),
    };
    Ok(json!({
        "protobuf_status": resp.status,
        "status_text": resp.status_text,
        "gateway": Value::Null,
        "server_message_id": get_field(&body_fields, 1, Some(0)).and_then(Field::as_varint).map(|v| v.to_string()),
        "business_status": get_field(&body_fields, 3, Some(0)).and_then(Field::as_varint),
        "client_message_id": text_value(get_field(&body_fields, 4, Some(2))),
        "check_code": get_field(&body_fields, 5, Some(0)).and_then(Field::as_varint),
    }))
}
