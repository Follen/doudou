//! Target resolution, numbered selection parsing, time windows, and self-UID inference.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, FixedOffset, Local, Timelike as _};
use serde_json::{Map, Value, json};

use crate::error::{DoudouError, Result};
use crate::io::normalize_lookup;

/// Resolve a conversation by exact `conversation_id` or unique normalized `display_name` substring.
pub fn resolve_conversation(conversations: &[Value], target: &str) -> Result<Value> {
    let raw = target.trim();
    if raw.is_empty() {
        return Err(DoudouError::new(
            "MISSING_ARGUMENT",
            "Missing a conversation id or name.",
        ));
    }
    for item in conversations {
        if item
            .get("conversation_id")
            .and_then(value_as_string)
            .as_deref()
            == Some(raw)
        {
            return Ok(item.clone());
        }
    }
    let normalized = normalize_lookup(raw);
    let mut matches: Vec<(usize, &Value)> = Vec::new();
    for (index, item) in conversations.iter().enumerate() {
        let name = item
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or("");
        if normalize_lookup(name).contains(&normalized) {
            matches.push((index, item));
        }
    }
    if matches.is_empty() {
        return Err(DoudouError::new(
            "RECIPIENT_NOT_FOUND",
            format!("No existing conversation matches: {raw}."),
        )
        .with_details(json!({ "target": raw, "candidates": [] })));
    }
    if matches.len() > 1 {
        let candidates: Vec<Value> = matches
            .iter()
            .map(|(index, item)| {
                let mut map = item.as_object().cloned().unwrap_or_else(Map::new);
                map.insert("number".to_owned(), json!(index + 1));
                Value::Object(map)
            })
            .collect();
        return Err(DoudouError::new(
            "CONVERSATION_AMBIGUOUS",
            format!("More than one conversation matches: {raw}."),
        )
        .with_details(json!({ "target": raw, "candidates": candidates })));
    }
    Ok(matches[0].1.clone())
}

/// Resolve a collected message by exact `message_id` or unique normalized `text` substring.
pub fn resolve_message(messages: &[Value], target: &str) -> Result<Value> {
    let raw = target.trim();
    if raw.is_empty() {
        return Err(DoudouError::new(
            "MISSING_ARGUMENT",
            "Missing a message id or text.",
        ));
    }
    for item in messages {
        if item.get("message_id").and_then(value_as_string).as_deref() == Some(raw) {
            return Ok(item.clone());
        }
    }
    let normalized = normalize_lookup(raw);
    let matches: Vec<&Value> = messages
        .iter()
        .filter(|item| {
            let text = item.get("text").and_then(Value::as_str).unwrap_or("");
            normalize_lookup(text).contains(&normalized)
        })
        .collect();
    if matches.is_empty() {
        return Err(DoudouError::new(
            "MESSAGE_NOT_FOUND",
            format!("No collected message contains: {raw}."),
        )
        .with_details(json!({ "query": raw, "candidates": [] })));
    }
    if matches.len() > 1 {
        return Err(DoudouError::new(
            "MESSAGE_AMBIGUOUS",
            format!("More than one message contains: {raw}."),
        )
        .with_details(json!({ "query": raw, "candidates": matches })));
    }
    Ok(matches[0].clone())
}

/// Parse 1-based conversation selection strings such as `"1,3-5"`, `"all"`, or `"全部"`.
pub fn parse_selection(value: &str, count: usize) -> Result<Vec<usize>> {
    let text = value.trim();
    if matches!(text, "全部" | "all" | "ALL") {
        return Ok((1..=count).collect());
    }
    let mut selected = BTreeSet::new();
    for raw_token in text.split(',') {
        let token = raw_token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some((left, right)) = token.split_once('-') {
            let start = parse_index(left)?;
            let end = parse_index(right)?;
            if start > end {
                return Err(DoudouError::new(
                    "INVALID_CONVERSATION_RANGE",
                    "Selection range starts after it ends.",
                ));
            }
            for n in start..=end {
                selected.insert(n);
            }
        } else {
            selected.insert(parse_index(token)?);
        }
    }
    let first = selected.first().copied();
    let last = selected.last().copied();
    if selected.is_empty() || first.is_some_and(|v| v < 1) || last.is_some_and(|v| v > count) {
        return Err(DoudouError::new(
            "CONVERSATION_NUMBER_OUT_OF_RANGE",
            "Selection contains an out-of-range conversation number.",
        ));
    }
    Ok(selected.into_iter().collect())
}

fn parse_index(raw: &str) -> Result<usize> {
    let trimmed = raw.trim();
    let signed: i64 = trimmed.parse().map_err(|_| {
        DoudouError::new(
            "INVALID_CONVERSATION_RANGE",
            format!("Invalid conversation number: {trimmed}."),
        )
    })?;
    if signed < 1 {
        return Err(DoudouError::new(
            "CONVERSATION_NUMBER_OUT_OF_RANGE",
            "Selection contains an out-of-range conversation number.",
        ));
    }
    usize::try_from(signed).map_err(|_| {
        DoudouError::new(
            "CONVERSATION_NUMBER_OUT_OF_RANGE",
            "Selection contains an out-of-range conversation number.",
        )
    })
}

/// Default collection window from local midnight to `clock`.
pub fn default_message_window(clock: DateTime<FixedOffset>) -> (String, String) {
    let start = clock
        .with_hour(0)
        .and_then(|d| d.with_minute(0))
        .and_then(|d| d.with_second(0))
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(clock);
    (start.to_rfc3339(), clock.to_rfc3339())
}

/// Default collection window using the current local clock.
pub fn default_local_message_window() -> (String, String) {
    default_message_window(Local::now().fixed_offset())
}

/// Infer the logged-in account's `uid` as the uniquely most frequent participant across direct chats.
pub fn infer_self_uid(conversations: &[Value]) -> Option<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for conversation in conversations {
        let conv_type = conversation
            .get("conversation_type")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if conv_type != 1 {
            continue;
        }
        let participants = conversation
            .get("participants")
            .and_then(|p| p.get("participants"))
            .and_then(Value::as_array);
        if let Some(list) = participants {
            for participant in list {
                if let Some(uid) = participant.get("uid").and_then(value_as_string)
                    && !uid.is_empty()
                {
                    *counts.entry(uid).or_insert(0) += 1;
                }
            }
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if ranked.is_empty() || (ranked.len() > 1 && ranked[0].1 == ranked[1].1) {
        return None;
    }
    Some(ranked[0].0.clone())
}

fn value_as_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}
