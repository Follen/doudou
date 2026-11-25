//! Chat read services: session index refresh (established + stranger inbox),
//! conversation listing, message collection, image decryption, and group member lookup.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, Utc};
use doudou_cdp::{AuthPolicy, ChromeSession, FetchRequest, IM_USER_INFO_PATH_MARKER};
use doudou_core::{
    DoudouError, IdentityCache, Result, enrich_message_identities, infer_self_uid,
    known_identity_sec_uids, normalize_identity, profile_paths, random_id, resolve_conversation,
    utc_now,
};
use doudou_protocol::{
    FetchResponse as ProtoFetchResponse, ResponseCodes, build_command_request, decode_fields,
    download_message_images, extract_messages, find_conversations, get_field, messages_request,
    parse_participants_page, parse_stranger_conversations, parse_stranger_messages,
    participants_request, response_body, stranger_conversations_request, stranger_messages_request,
    stranger_request, stranger_response_body, web_path,
};
use doudou_storage::{
    merge_messages, read_conversation_index, read_messages, read_session_index_snapshot,
    unknown_field_inventory, write_conversation_index, write_session_index_snapshot,
};
use serde_json::{Map, Value, json};

use crate::reads::transport::{parse_result, to_protocol_fetch_response};

/// Cached conversation index freshness window (2 hours).
pub const INDEX_MAX_AGE_SECONDS: f64 = 2.0 * 60.0 * 60.0;

/// Douyin IM endpoint for fetching established conversation messages (cmd 301).
pub const GET_MESSAGES_ENDPOINT: &str = "https://imapi.douyin.com/v1/message/get_by_conversation";

/// Douyin IM endpoint for fetching group participant lists (cmd 605).
pub const PARTICIPANTS_ENDPOINT: &str =
    "https://imapi.douyin.com/v1/conversation/participants_list";

/// Douyin IM endpoint for fetching stranger message-request conversations (cmd 1001).
pub const STRANGER_CONVERSATIONS_ENDPOINT: &str =
    "https://imapi.douyin.com/v1/stranger/get_conversation_list";

/// Douyin IM endpoint for fetching stranger message-request messages (cmd 1002).
pub const STRANGER_MESSAGES_ENDPOINT: &str = "https://imapi.douyin.com/v1/stranger/get_messages";

fn response_identities(root: &Value) -> Vec<Value> {
    let mut found = Vec::new();
    let mut stack: Vec<(&Value, usize)> = vec![(root, 0)];
    while let Some((current, depth)) = stack.pop() {
        if depth > 6 {
            continue;
        }
        match current {
            Value::Object(map) => {
                let identity = normalize_identity(Some(current), "user-info");
                let has_uid = identity
                    .get("uid")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty());
                let has_sec = identity
                    .get("sec_uid")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty());
                if has_uid || has_sec {
                    found.push(identity);
                }
                for child in map.values() {
                    stack.push((child, depth + 1));
                }
            }
            Value::Array(items) => {
                for child in items {
                    stack.push((child, depth + 1));
                }
            }
            _ => {}
        }
    }
    found
}

fn value_as_non_empty_str(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn value_as_i64(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => s.parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

fn value_as_u64(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

fn merge_json_objects(base: &Value, overlay: &Value) -> Value {
    let mut out = base.as_object().cloned().unwrap_or_default();
    if let Some(overlay_map) = overlay.as_object() {
        for (k, v) in overlay_map {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

fn captured_user_info_endpoint(session: &ChromeSession) -> Option<String> {
    session
        .captured_exchanges()
        .into_iter()
        .rev()
        .find(|ex| ex.url.contains(IM_USER_INFO_PATH_MARKER))
        .map(|ex| ex.url)
}

/// Resolve structured Douyin user identities for `sec_uids` using captured IM batches,
/// live `/aweme/v1/web/im/user/info/` lookups, and the persistent [`IdentityCache`].
pub async fn resolve_user_identities(
    session: &mut ChromeSession,
    home: &Path,
    data_root: &Path,
    profile: &str,
    sec_uids: &[String],
) -> Result<HashMap<String, Value>> {
    let mut identities: HashMap<String, Value> = HashMap::new();
    let mut cache = IdentityCache::load(home, data_root, profile)?;

    let mut unique: Vec<String> = Vec::new();
    let mut seen_uid = HashSet::new();
    for s in sec_uids {
        if !s.is_empty() && seen_uid.insert(s.clone()) {
            unique.push(s.clone());
        }
    }
    if unique.is_empty() {
        return Ok(identities);
    }

    for raw in session.captured_user_info_responses() {
        let Ok(parsed) = serde_json::from_slice::<Value>(&raw) else {
            continue;
        };
        let status_code = parsed
            .get("status_code")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if !parsed.is_object() || status_code != 0 {
            continue;
        }
        for identity in response_identities(&parsed) {
            if let Some(sec_uid) = identity.get("sec_uid").and_then(Value::as_str)
                && seen_uid.contains(sec_uid)
            {
                let merged = cache.merge(&identity, "page-user-info");
                identities.insert(sec_uid.to_owned(), merged);
            }
        }
    }

    let missing: Vec<String> = unique
        .iter()
        .filter(|sec_uid| !identities.contains_key(sec_uid.as_str()))
        .cloned()
        .collect();

    if !missing.is_empty() {
        let endpoint = captured_user_info_endpoint(session).unwrap_or_else(|| {
            let path = web_path("user_info").unwrap_or("/aweme/v1/web/im/user/info/");
            format!("https://www.douyin.com{path}")
        });
        for chunk in missing.chunks(20) {
            let encoded_ids = serde_json::to_string(chunk).unwrap_or_else(|_| "[]".to_owned());
            let form_body = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("sec_user_ids", &encoded_ids)
                .finish();
            let req = FetchRequest::post_bytes(
                &endpoint,
                "application/x-www-form-urlencoded",
                form_body.as_bytes(),
            )
            .with_response_kind(doudou_cdp::FetchResponseKind::Text);
            let Ok(raw_resp) = session.page_fetch(&req).await else {
                continue;
            };
            let proto_resp: ProtoFetchResponse = to_protocol_fetch_response(&raw_resp);
            let parsed_map = match parse_result(
                session,
                &proto_resp,
                ResponseCodes {
                    empty: "USER_INFO_RESPONSE_EMPTY",
                    invalid: "USER_INFO_RESPONSE_INVALID",
                    failed: "USER_INFO_LOOKUP_FAILED",
                },
                "chat.user_info",
            )
            .await
            {
                Ok(m) => m,
                Err(err)
                    if matches!(
                        err.code.as_str(),
                        "LOGIN_REQUIRED" | "VERIFICATION_REQUIRED"
                    ) =>
                {
                    return Err(err);
                }
                Err(_) => continue,
            };
            for identity in response_identities(&Value::Object(parsed_map)) {
                if let Some(sec_uid) = identity.get("sec_uid").and_then(Value::as_str) {
                    let merged = cache.merge(&identity, "user-info");
                    identities.insert(sec_uid.to_owned(), merged);
                }
            }
        }
    }

    for sec_uid in &unique {
        if !identities.contains_key(sec_uid.as_str()) {
            let enriched = cache.enrich(&json!({ "sec_uid": sec_uid }), "user-info-cache");
            identities.insert(sec_uid.clone(), enriched);
        }
    }
    cache.save()?;
    Ok(identities)
}

/// Apply pre-resolved identities onto `conversations`, deriving direct-chat `display_name`
/// from the counterparty only when `self_uid` is unambiguously known.
pub fn apply_conversation_identities<S: std::hash::BuildHasher>(
    conversations: &mut [Value],
    identities: &HashMap<String, Value, S>,
) {
    let self_uid = infer_self_uid(conversations);

    for conversation in conversations.iter_mut() {
        if let Some(participants) = conversation
            .pointer_mut("/participants/participants")
            .and_then(Value::as_array_mut)
        {
            for participant in participants.iter_mut() {
                if let Some(sec_uid) = participant.get("sec_uid").and_then(Value::as_str)
                    && let Some(resolved) = identities.get(sec_uid)
                {
                    *participant = merge_json_objects(participant, resolved);
                }
            }
        }

        if let Some(last_message) = conversation
            .get_mut("last_message")
            .and_then(Value::as_object_mut)
        {
            let sender_val = last_message
                .get("sender")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let mut merged_sender = if let Some(sec_uid) =
                sender_val.get("sec_uid").and_then(Value::as_str)
                && let Some(resolved) = identities.get(sec_uid)
            {
                merge_json_objects(&sender_val, resolved)
            } else {
                sender_val
            };
            if let Some(sender_obj) = merged_sender.as_object_mut() {
                let display = sender_obj
                    .get("display_name")
                    .cloned()
                    .unwrap_or(Value::Null);
                sender_obj.insert("name".to_owned(), display);
            }
            last_message.insert("sender".to_owned(), merged_sender);
        }

        let conv_type = conversation
            .get("conversation_type")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let is_direct =
            conv_type == 1 || conversation.get("kind").and_then(Value::as_str) == Some("direct");
        if !is_direct {
            continue;
        }

        // Direct conversations are addressable by the other participant only when self_uid is
        // strictly inferred; never replace the title with the current account's identity.
        let Some(ref own_uid) = self_uid else {
            continue;
        };
        let participants = conversation
            .pointer("/participants/participants")
            .and_then(Value::as_array);
        let Some(participants) = participants else {
            continue;
        };
        let names: Vec<String> = participants
            .iter()
            .filter(|p| value_as_non_empty_str(p.get("uid")).as_deref() != Some(own_uid.as_str()))
            .filter_map(|p| {
                for key in ["remark_name", "nickname", "display_name", "alias"] {
                    if let Some(val) = p
                        .get(key)
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        return Some(val.to_owned());
                    }
                }
                None
            })
            .collect();
        if names.len() == 1
            && let Some(conv_obj) = conversation.as_object_mut()
        {
            conv_obj.insert("display_name".to_owned(), json!(names[0]));
        }
    }
}

/// Resolve and apply identities for all participants and last-message senders in `conversations`.
pub async fn enrich_conversation_identities(
    session: &mut ChromeSession,
    home: &Path,
    data_root: &Path,
    profile: &str,
    conversations: &mut [Value],
) -> Result<()> {
    let mut targets: Vec<String> = Vec::new();
    for conversation in conversations.iter() {
        if let Some(participants) = conversation
            .pointer("/participants/participants")
            .and_then(Value::as_array)
        {
            for participant in participants {
                if let Some(sec_uid) = participant.get("sec_uid").and_then(Value::as_str) {
                    targets.push(sec_uid.to_owned());
                }
            }
        }
        if let Some(sec_uid) = conversation
            .pointer("/last_message/sender/sec_uid")
            .and_then(Value::as_str)
        {
            targets.push(sec_uid.to_owned());
        }
    }
    let identities = resolve_user_identities(session, home, data_root, profile, &targets).await?;
    apply_conversation_identities(conversations, &identities);
    Ok(())
}

fn age_seconds(timestamp: Option<&str>) -> Option<f64> {
    let raw = timestamp?;
    let parsed = DateTime::parse_from_rfc3339(raw).ok()?;
    let delta = Utc::now().signed_duration_since(parsed.with_timezone(&Utc));
    Some((delta.num_milliseconds().max(0) as f64) / 1000.0)
}

/// Refresh (or return cached) conversation index merging established and stranger inboxes.
///
/// If stranger conversation pagination fails or stalls, the previously saved `conversations.json`
/// index is left untouched.
pub async fn refresh_session_index(
    session: &mut ChromeSession,
    home: &Path,
    data_root: &Path,
    profile: &str,
    force: bool,
) -> Result<Value> {
    let cached = read_session_index_snapshot(home, data_root, profile)?;
    let age = cached
        .as_ref()
        .and_then(|c| c.get("completed_at").and_then(Value::as_str))
        .and_then(|ts| age_seconds(Some(ts)));

    if !force
        && let Some(ref snap) = cached
        && snap.get("inbox_count").is_some()
        && let Some(a) = age
        && a <= INDEX_MAX_AGE_SECONDS
    {
        let _ = session.ensure_authenticated(AuthPolicy::FailFast).await?;
        if let Some(index) = read_conversation_index(home, data_root, profile)? {
            return Ok(json!({
                "source": "cache",
                "age_seconds": a,
                "snapshot": snap,
                "index": index,
            }));
        }
    }

    let _login = session.ensure_authenticated(AuthPolicy::FailFast).await?;
    let (init_request, init_response) = session.captured_im_init().ok_or_else(|| {
        DoudouError::new(
            "IM_INITIALIZATION_MISSING",
            "IM initialization request or response envelope is missing.",
        )
    })?;

    let mut conversations = find_conversations(&init_response);

    // Fetch stranger message requests via cmd 1001; merge only after every page succeeds so
    // any protocol failure preserves the existing conversation index.
    let mut stranger_by_id: BTreeMap<String, Value> = BTreeMap::new();
    let mut cursor: u64 = 0;
    let mut seen: HashSet<u64> = HashSet::new();
    let mut completed_stranger = false;

    for _ in 0..100 {
        if !seen.insert(cursor) {
            return Err(DoudouError::new(
                "STRANGER_PAGE_CURSOR_STALLED",
                "Stranger inbox cursor did not advance.",
            ));
        }
        let req_bytes = stranger_request(
            &init_request,
            1001,
            &stranger_conversations_request(cursor, 50, true),
            None,
        )?;
        let (http_status, raw) = session
            .post_protobuf(STRANGER_CONVERSATIONS_ENDPOINT, &req_bytes)
            .await?;
        let im_resp = stranger_response_body(&raw, 1001)?;
        if im_resp.gateway.is_some() {
            return Err(DoudouError::new(
                "VERIFICATION_REQUIRED",
                "Stranger conversation request reached a verification gateway.",
            ));
        }
        let Some(payload) = im_resp.body else {
            return Err(DoudouError::new(
                "STRANGER_CONVERSATIONS_FAILED",
                "Stranger conversation protocol request failed.",
            )
            .with_details(json!({
                "http_status": http_status,
                "status": im_resp.status,
                "status_text": im_resp.status_text,
            })));
        };
        if http_status != 200 || im_resp.status != 0 {
            return Err(DoudouError::new(
                "STRANGER_CONVERSATIONS_FAILED",
                "Stranger conversation protocol request failed.",
            )
            .with_details(json!({
                "http_status": http_status,
                "status": im_resp.status,
                "status_text": im_resp.status_text,
            })));
        }

        let page = parse_stranger_conversations(&payload);
        if let Some(items) = page.get("conversations").and_then(Value::as_array) {
            for item in items {
                if let Some(cid) = value_as_non_empty_str(item.get("conversation_id")) {
                    stranger_by_id.insert(cid, item.clone());
                }
            }
        }
        let has_more = page
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !has_more {
            completed_stranger = true;
            break;
        }
        let next_cursor_str =
            value_as_non_empty_str(page.get("next_cursor")).unwrap_or_else(|| "0".to_owned());
        if next_cursor_str == cursor.to_string() {
            return Err(DoudouError::new(
                "STRANGER_PAGE_CURSOR_STALLED",
                "Stranger inbox cursor did not advance.",
            )
            .with_details(json!({ "cursor": cursor.to_string() })));
        }
        cursor = next_cursor_str.parse::<u64>().map_err(|_| {
            DoudouError::new(
                "PROTOCOL_SCHEMA_DRIFTED",
                "Stranger inbox returned a non-numeric cursor.",
            )
            .with_details(json!({ "cursor": next_cursor_str }))
        })?;
    }

    if !completed_stranger {
        return Err(DoudouError::new(
            "STRANGER_PAGE_LIMIT",
            "Stranger conversation collection reached the page limit.",
        ));
    }

    conversations.extend(stranger_by_id.into_values());
    if conversations.is_empty() {
        return Err(DoudouError::new(
            "IM_INITIALIZATION_EMPTY",
            "IM initialization and stranger inbox returned no conversations.",
        ));
    }

    enrich_conversation_identities(session, home, data_root, profile, &mut conversations).await?;
    let index = write_conversation_index(home, data_root, profile, &conversations)?;
    let status = session.status().await;
    let direct_count = conversations
        .iter()
        .filter(|c| c.get("kind").and_then(Value::as_str) == Some("direct"))
        .count();
    let group_count = conversations
        .iter()
        .filter(|c| c.get("kind").and_then(Value::as_str) == Some("group"))
        .count();
    let inbox_count = conversations
        .iter()
        .filter(|c| c.get("inbox").and_then(Value::as_str) == Some("request"))
        .count();

    let paths = profile_paths(home, data_root, profile)?;
    let snapshot = json!({
        "schema": "doudou.session-index-snapshot.v1",
        "profile_id": paths.profile_id,
        "completed_at": utc_now(),
        "conversation_count": conversations.len(),
        "direct_count": direct_count,
        "group_count": group_count,
        "inbox_count": inbox_count,
        "browser_initialization_count": status.context_generation,
    });
    let files = write_session_index_snapshot(home, data_root, profile, &snapshot)?;

    Ok(json!({
        "source": "live",
        "age_seconds": 0.0,
        "snapshot": snapshot,
        "index": index,
        "files": files,
    }))
}

/// Filter a conversation index slice by `kind` (`all|direct|group`) and `inbox` (`all|established|request`).
pub fn filter_conversations(conversations: &[Value], kind: &str, inbox: &str) -> Vec<Value> {
    conversations
        .iter()
        .filter(|item| {
            let kind_ok = kind == "all" || item.get("kind").and_then(Value::as_str) == Some(kind);
            let item_inbox = item
                .get("inbox")
                .and_then(Value::as_str)
                .unwrap_or("established");
            let inbox_ok = inbox == "all" || item_inbox == inbox;
            kind_ok && inbox_ok
        })
        .cloned()
        .collect()
}

/// List conversations for `profile`, refreshing the session index if stale or forced.
pub async fn list_conversations(
    session: &mut ChromeSession,
    home: &Path,
    data_root: &Path,
    profile: &str,
    force: bool,
    kind: &str,
    inbox: &str,
) -> Result<Value> {
    let refreshed = refresh_session_index(session, home, data_root, profile, force).await?;
    let all = refreshed
        .pointer("/index/conversations")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let filtered = filter_conversations(all, kind, inbox);
    Ok(json!({
        "status": "ok",
        "source": refreshed.get("source").cloned().unwrap_or(Value::Null),
        "refreshed_at": refreshed.pointer("/index/refreshed_at").cloned().unwrap_or(Value::Null),
        "count": filtered.len(),
        "conversations": filtered,
    }))
}

fn parse_rfc3339_ms(raw: &str) -> Result<i64> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
        .map_err(|err| {
            DoudouError::new(
                "INVALID_TIMESTAMP",
                format!("Invalid ISO-8601 timestamp '{raw}': {err}"),
            )
        })
}

/// Collect messages for `target` conversation (established cmd 301 or stranger cmd 1002),
/// enrich identities, optionally download and decrypt image attachments, and merge into storage.
#[allow(clippy::too_many_arguments)]
pub async fn collect_messages(
    session: &mut ChromeSession,
    home: &Path,
    data_root: &Path,
    profile: &str,
    target: &str,
    from_time: Option<&str>,
    to_time: Option<&str>,
    max_pages: usize,
    download_images: bool,
) -> Result<Value> {
    let refreshed = refresh_session_index(session, home, data_root, profile, false).await?;
    let conversations = refreshed
        .pointer("/index/conversations")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut conversation = resolve_conversation(conversations, target)?;
    let (init_request, _) = session.captured_im_init().ok_or_else(|| {
        DoudouError::new(
            "IM_INITIALIZATION_MISSING",
            "The current broker session has no IM request envelope.",
        )
    })?;

    let lower_ms = from_time.map(parse_rfc3339_ms).transpose()?;
    let upper_ms = to_time.map(parse_rfc3339_ms).transpose()?;

    let conv_id = value_as_non_empty_str(conversation.get("conversation_id")).ok_or_else(|| {
        DoudouError::new(
            "INVALID_CONVERSATION",
            "Conversation missing conversation_id.",
        )
    })?;
    let is_request = conversation.get("inbox").and_then(Value::as_str) == Some("request");

    let mut messages_by_id: HashMap<String, Value> = HashMap::new();
    let mut pages: Vec<Value> = Vec::new();
    let mut anchor: i64 = 0;
    let mut reached_end = false;

    for page_number in 1..=max_pages {
        let (endpoint, command, request_bytes) = if is_request {
            let short_id = value_as_u64(
                conversation
                    .get("conversation_short_id")
                    .or_else(|| conversation.get("conversation_id")),
            );
            let body = stranger_messages_request(short_id, false);
            let req = stranger_request(&init_request, 1002, &body, None)?;
            (STRANGER_MESSAGES_ENDPOINT, 1002u32, req)
        } else {
            let conv_type = conversation
                .get("conversation_type")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            let short_id = value_as_u64(conversation.get("conversation_short_id"));
            let body = messages_request(&conv_id, conv_type, short_id, anchor.max(0) as u64, 50, 1);
            let req = build_command_request(&init_request, 301, &body, None)?;
            (GET_MESSAGES_ENDPOINT, 301u32, req)
        };

        let (http_status, raw) = session.post_protobuf(endpoint, &request_bytes).await?;
        let im_resp = if command == 1002 {
            stranger_response_body(&raw, command)?
        } else {
            response_body(&raw, command)?
        };
        if let Some(gateway) = im_resp.gateway {
            let mut keys: Vec<String> = gateway
                .as_object()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            keys.sort();
            return Err(DoudouError::new(
                "VERIFICATION_REQUIRED",
                "Message collection reached a JSON verification gateway.",
            )
            .with_details(json!({ "gateway_keys": keys })));
        }
        let Some(payload) = im_resp.body else {
            return Err(DoudouError::new(
                "MESSAGE_COLLECT_FAILED",
                "Message protocol request failed.",
            )
            .with_details(json!({
                "http_status": http_status,
                "status": im_resp.status,
                "status_text": im_resp.status_text,
            })));
        };
        if http_status != 200 || im_resp.status != 0 {
            return Err(DoudouError::new(
                "MESSAGE_COLLECT_FAILED",
                "Message protocol request failed.",
            )
            .with_details(json!({
                "http_status": http_status,
                "status": im_resp.status,
                "status_text": im_resp.status_text,
            })));
        }

        let mut batch = if command == 1002 {
            let mut parsed = parse_stranger_messages(&payload, Some(&conv_id));
            for item in &mut parsed {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("collection_source".to_owned(), json!("stranger-protocol"));
                }
            }
            parsed
        } else {
            extract_messages(&payload, &conv_id)
        };

        let protocol_received = batch.len();
        let mut summary_received = 0usize;
        let mut collection_source = if batch.is_empty() {
            "empty"
        } else {
            "protocol"
        };

        if command == 1002
            && batch.is_empty()
            && let Some(summary) = conversation.get("last_message").and_then(Value::as_object)
            && value_as_non_empty_str(summary.get("message_id")).is_some()
        {
            let mut copy = summary.clone();
            copy.insert(
                "collection_source".to_owned(),
                json!("stranger-conversation-summary"),
            );
            batch.push(Value::Object(copy));
            summary_received = 1;
            collection_source = "conversation_summary";
        }

        let mut oldest: Option<i64> = None;
        for item in &batch {
            let created = value_as_i64(item.get("create_time_ms"));
            oldest = Some(oldest.map_or(created, |cur| cur.min(created)));
            let in_lower = lower_ms.is_none_or(|lo| created >= lo);
            let in_upper = upper_ms.is_none_or(|hi| created <= hi);
            if in_lower
                && in_upper
                && let Some(mid) = value_as_non_empty_str(item.get("message_id"))
            {
                messages_by_id.insert(mid, item.clone());
            }
        }

        let (next_cursor, has_more) = if command == 1002 {
            (0i64, false)
        } else {
            let fields = decode_fields(&payload)?;
            let next = get_field(&fields, 2, Some(0))
                .and_then(doudou_protocol::Field::as_varint)
                .map_or(0, |v| v as i64);
            let more = get_field(&fields, 3, Some(0))
                .and_then(doudou_protocol::Field::as_varint)
                .is_some_and(|v| v != 0);
            (next, more)
        };

        pages.push(json!({
            "page": page_number,
            "anchor_in": anchor.to_string(),
            "next_cursor": next_cursor.to_string(),
            "received": batch.len(),
            "protocol_received": protocol_received,
            "summary_received": summary_received,
            "collection_source": collection_source,
            "fallback_used": false,
            "protocol_http_status": http_status,
            "protocol_status": im_resp.status,
            "payload_bytes": payload.len(),
            "has_more": has_more,
        }));

        if !has_more
            || next_cursor == anchor
            || (lower_ms.is_some() && oldest.is_some_and(|old| old < lower_ms.unwrap_or(0)))
        {
            reached_end = true;
            break;
        }
        anchor = next_cursor;
    }

    if !reached_end {
        return Err(DoudouError::new(
            "MESSAGE_PAGE_LIMIT",
            "Message collection reached the configured page limit.",
        )
        .with_details(json!({ "max_pages": max_pages })));
    }

    let mut ordered: Vec<Value> = messages_by_id.into_values().collect();
    ordered.sort_by(|left, right| {
        let key = |item: &Value| {
            let ts = value_as_i64(item.get("create_time_ms"));
            let mid_num = value_as_non_empty_str(item.get("message_id"))
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            (ts, mid_num)
        };
        key(left).cmp(&key(right))
    });

    let sec_uids = known_identity_sec_uids(&Value::Array(ordered.clone()));
    let identities = resolve_user_identities(session, home, data_root, profile, &sec_uids).await?;
    for message in &mut ordered {
        if let Some(obj) = message.as_object_mut() {
            let sender_val = obj.get("sender").cloned().unwrap_or_else(|| json!({}));
            let mut merged = if let Some(sec_uid) =
                sender_val.get("sec_uid").and_then(Value::as_str)
                && let Some(resolved) = identities.get(sec_uid)
            {
                merge_json_objects(&sender_val, resolved)
            } else {
                sender_val
            };
            if let Some(sender_obj) = merged.as_object_mut() {
                let display = sender_obj
                    .get("display_name")
                    .cloned()
                    .unwrap_or(Value::Null);
                sender_obj.insert("name".to_owned(), display);
            }
            obj.insert("sender".to_owned(), merged);
        }
    }
    if let Some(last_msg) = conversation
        .get_mut("last_message")
        .and_then(Value::as_object_mut)
    {
        let sender_val = last_msg.get("sender").cloned().unwrap_or_else(|| json!({}));
        let mut merged = if let Some(sec_uid) = sender_val.get("sec_uid").and_then(Value::as_str)
            && let Some(resolved) = identities.get(sec_uid)
        {
            merge_json_objects(&sender_val, resolved)
        } else {
            sender_val
        };
        if let Some(sender_obj) = merged.as_object_mut() {
            let display = sender_obj
                .get("display_name")
                .cloned()
                .unwrap_or(Value::Null);
            sender_obj.insert("name".to_owned(), display);
        }
        last_msg.insert("sender".to_owned(), merged);
    }

    let mut cache = IdentityCache::load(home, data_root, profile)?;
    let reference_catalog = read_messages(home, data_root, profile, &conversation)?;
    ordered = enrich_message_identities(&ordered, &cache, "message-collect", &reference_catalog);
    for msg in &ordered {
        if let Some(sender) = msg.get("sender") {
            let _ = cache.merge(sender, "message-collect");
        }
    }
    cache.save()?;

    let mut attempted = 0usize;
    let mut downloaded = 0usize;
    let mut failed = 0usize;
    if download_images {
        let paths = profile_paths(home, data_root, profile)?;
        for message in &mut ordered {
            if message.get("message_type").and_then(Value::as_u64) != Some(27) {
                continue;
            }
            let ts_ms = value_as_i64(message.get("create_time_ms"));
            let day = DateTime::from_timestamp_millis(ts_ms).map_or_else(
                || "1970-01-01".to_owned(),
                |dt| dt.format("%Y-%m-%d").to_string(),
            );
            let mid = value_as_non_empty_str(message.get("message_id"))
                .unwrap_or_else(|| "unknown".to_owned());
            let directory = paths.conversations.join("media").join(day).join(mid);
            let results = download_message_images(message, &directory).await;
            attempted += results.len();
            downloaded += results
                .iter()
                .filter(|r| r.get("status").and_then(Value::as_str) == Some("downloaded"))
                .count();
            failed += results
                .iter()
                .filter(|r| r.get("status").and_then(Value::as_str) != Some("downloaded"))
                .count();
            if let Some(obj) = message.as_object_mut() {
                obj.insert("images".to_owned(), Value::Array(results));
            }
        }
    }

    let merge = merge_messages(home, data_root, profile, &conversation, &ordered)?;
    let inventory = unknown_field_inventory(&ordered);

    Ok(json!({
        "schema": "doudou.message-collect-run.v1",
        "run_id": random_id("collect"),
        "completed_at": utc_now(),
        "conversation": conversation,
        "pages": pages,
        "message_count": ordered.len(),
        "unknown_field_inventory": inventory,
        "images": {
            "attempted": attempted,
            "downloaded": downloaded,
            "failed": failed,
        },
        "merge": merge,
    }))
}

/// Resolve group members for `requested` queries using cmd 605 participant pagination.
pub async fn resolve_members(
    session: &mut ChromeSession,
    home: &Path,
    data_root: &Path,
    profile: &str,
    conversation: &Value,
    requested: &[String],
) -> Result<Vec<Value>> {
    if requested.is_empty() {
        return Ok(Vec::new());
    }
    if conversation
        .get("conversation_type")
        .and_then(Value::as_u64)
        != Some(2)
    {
        return Err(DoudouError::new(
            "MENTION_REQUIRES_GROUP",
            "Mentions are only valid in group conversations.",
        ));
    }
    let (init_request, _) = session.captured_im_init().ok_or_else(|| {
        DoudouError::new(
            "IM_INITIALIZATION_MISSING",
            "The current broker session has no IM request envelope.",
        )
    })?;

    let initial = conversation
        .get("participants")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut members: Vec<Value> = initial
        .get("participants")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut cursor = value_as_u64(initial.get("cursor"));
    let mut has_more = initial
        .get("has_more")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    for _ in 0..100 {
        if !has_more {
            break;
        }
        let conv_id =
            value_as_non_empty_str(conversation.get("conversation_id")).unwrap_or_default();
        let short_id = value_as_u64(conversation.get("conversation_short_id"));
        let conv_type = conversation
            .get("conversation_type")
            .and_then(Value::as_u64)
            .unwrap_or(2);
        let body = participants_request(&conv_id, short_id, conv_type, cursor, 100);
        let req = build_command_request(&init_request, 605, &body, None)?;
        let (http_status, raw) = session.post_protobuf(PARTICIPANTS_ENDPOINT, &req).await?;
        let im_resp = response_body(&raw, 605)?;
        if im_resp.gateway.is_some() {
            return Err(DoudouError::new(
                "VERIFICATION_REQUIRED",
                "Member lookup reached a JSON verification gateway.",
            ));
        }
        let Some(payload) = im_resp.body else {
            return Err(DoudouError::new(
                "PARTICIPANT_LIST_FAILED",
                "Group member protocol request failed.",
            )
            .with_details(json!({
                "http_status": http_status,
                "status": im_resp.status,
                "status_text": im_resp.status_text,
            })));
        };
        if http_status != 200 || im_resp.status != 0 {
            return Err(DoudouError::new(
                "PARTICIPANT_LIST_FAILED",
                "Group member protocol request failed.",
            )
            .with_details(json!({
                "http_status": http_status,
                "status": im_resp.status,
                "status_text": im_resp.status_text,
            })));
        }
        let wrapper = decode_fields(&payload)?;
        let page_bytes = get_field(&wrapper, 1, Some(2))
            .and_then(|f| f.as_bytes())
            .unwrap_or(&[]);
        let page = parse_participants_page(page_bytes);
        if let Some(items) = page.get("participants").and_then(Value::as_array) {
            members.extend(items.iter().cloned());
        }
        cursor = value_as_u64(page.get("cursor"));
        has_more = page
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }

    let mut by_uid: BTreeMap<String, Value> = BTreeMap::new();
    for member in members {
        if let Some(uid) = value_as_non_empty_str(member.get("uid")) {
            by_uid.insert(uid, member);
        }
    }
    let mut unique_members: Vec<Value> = by_uid.into_values().collect();
    let sec_uids: Vec<String> = unique_members
        .iter()
        .filter_map(|m| {
            m.get("sec_uid")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect();
    let identities = resolve_user_identities(session, home, data_root, profile, &sec_uids).await?;
    for member in &mut unique_members {
        if let Some(sec_uid) = member.get("sec_uid").and_then(Value::as_str)
            && let Some(resolved) = identities.get(sec_uid)
        {
            *member = merge_json_objects(member, resolved);
        }
        if let Some(obj) = member.as_object_mut() {
            let name = obj
                .get("display_name")
                .and_then(Value::as_str)
                .or_else(|| obj.get("alias").and_then(Value::as_str))
                .unwrap_or("")
                .to_owned();
            obj.insert("name".to_owned(), json!(name));
        }
    }

    let mut resolved_by_uid: BTreeMap<String, Value> = BTreeMap::new();
    for query in requested {
        let matches: Vec<&Value> = unique_members
            .iter()
            .filter(|m| {
                ["name", "alias", "uid", "sec_uid"]
                    .iter()
                    .any(|k| value_as_non_empty_str(m.get(*k)).as_deref() == Some(query.as_str()))
            })
            .collect();
        if matches.is_empty() {
            return Err(DoudouError::new(
                "MENTION_MEMBER_NOT_FOUND",
                "Group member was not found.",
            )
            .with_details(json!({ "query": query })));
        }
        if matches.len() > 1 {
            let candidates: Vec<Value> = matches
                .iter()
                .map(|m| {
                    let mut map = Map::new();
                    map.insert(
                        "uid".to_owned(),
                        m.get("uid").cloned().unwrap_or(Value::Null),
                    );
                    map.insert(
                        "name".to_owned(),
                        m.get("name").cloned().unwrap_or(Value::Null),
                    );
                    Value::Object(map)
                })
                .collect();
            return Err(DoudouError::new(
                "MENTION_MEMBER_AMBIGUOUS",
                "Group member name is ambiguous.",
            )
            .with_details(json!({
                "query": query,
                "candidates": candidates,
            })));
        }
        let chosen = matches[0];
        if let Some(uid) = value_as_non_empty_str(chosen.get("uid")) {
            resolved_by_uid.insert(uid, chosen.clone());
        }
    }

    Ok(resolved_by_uid.into_values().collect())
}
