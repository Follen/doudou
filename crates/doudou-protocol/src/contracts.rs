//! Versioned Douyin Web API endpoint paths and field alias contracts.

use doudou_core::{DoudouError, Result};
use serde_json::{Map, Value, json};

pub const WEB_PROTOCOL_VERSION: &str = "2026-08-02.1";

/// Versioned Web API endpoint lookup.
pub fn web_path(key: &str) -> Option<&'static str> {
    match key {
        "video_list" => Some("/aweme/v1/web/aweme/post/"),
        "video_detail" => Some("/aweme/v1/web/aweme/detail/"),
        "comment_root" => Some("/aweme/v1/web/comment/list/"),
        "comment_reply" => Some("/aweme/v1/web/comment/list/reply/"),
        "comment_publish" => Some("/aweme/v1/web/comment/publish/"),
        "comment_digg" => Some("/aweme/v1/web/comment/digg/"),
        "user_info" => Some("/aweme/v1/web/im/user/info/"),
        _ => None,
    }
}

/// Field aliases registered in the versioned Web protocol contract.
pub fn field_aliases(key: &str) -> &'static [&'static str] {
    match key {
        "status_code" => &["status_code", "statusCode"],
        "video_items" => &["aweme_list", "awemeList"],
        "video_cursor" => &["max_cursor", "maxCursor"],
        "has_more" => &["has_more", "hasMore"],
        "video_detail" => &["aweme_detail", "awemeDetail"],
        "comments" => &["comments"],
        "comment_cursor" => &["cursor"],
        "embedded_replies" => &["reply_comment", "reply_comments"],
        _ => &[],
    }
}

/// Extract the first present aliased field from a JSON object payload.
pub fn contract_value<'a>(payload: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    for alias in field_aliases(key) {
        if let Some(val) = payload.get(*alias) {
            return Some(val);
        }
    }
    None
}

/// Extract a list field under the versioned contract, failing with `PROTOCOL_SCHEMA_DRIFTED` on type drift.
pub fn contract_list<'a>(
    payload: &'a Map<String, Value>,
    key: &str,
    operation: &str,
) -> Result<&'a [Value]> {
    match contract_value(payload, key) {
        Some(Value::Null) => Ok(&[]),
        Some(Value::Array(items)) => Ok(items.as_slice()),
        other => {
            let actual_type = match other {
                None => "missing",
                Some(Value::Bool(_)) => "bool",
                Some(Value::Number(_)) => "number",
                Some(Value::String(_)) => "str",
                Some(Value::Object(_)) => "dict",
                Some(Value::Array(_) | Value::Null) => unreachable!(),
            };
            let mut keys: Vec<&String> = payload.keys().collect();
            keys.sort();
            Err(DoudouError::new(
                "PROTOCOL_SCHEMA_DRIFTED",
                "A versioned protocol response no longer matches the expected field contract.",
            )
            .with_details(json!({
                "contract_version": WEB_PROTOCOL_VERSION,
                "operation": operation,
                "field": key,
                "aliases": field_aliases(key),
                "response_keys": keys,
                "actual_type": actual_type,
            })))
        }
    }
}
