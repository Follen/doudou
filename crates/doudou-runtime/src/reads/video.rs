//! Video read services: Douyin ID resolution, user video index collection,
//! and video detail + resumable checkpointed comment/reply collection.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use doudou_cdp::{AuthPolicy, CapturedNetworkExchange, ChromeSession};
use doudou_core::{DoudouError, IdentityCache, Result, enrich_known_identities};
use doudou_protocol::{
    FetchResponse as ProtoFetchResponse, ResponseCodes, contract_list, contract_value,
    identities_in, normalize_comment, normalize_video, normalize_video_detail, parse_render_data,
    resolve_comment_parents, web_path,
};
use doudou_storage::{
    merge_video_comments, read_comment_checkpoint, read_video_comments, write_comment_checkpoint,
    write_identity, write_video, write_video_index,
};
use doudou_ui::{
    ProfileVideoListComponent, UiPage as _, VideoCommentsPanelComponent, VideoDetailComponent,
};
use serde_json::{Map, Value, json};
use url::Url;

use crate::cdp_ui_bridge::CdpUiPage;
use crate::reads::transport::{fetch_json, parse_result};

const COMMENT_CODES: ResponseCodes = ResponseCodes {
    empty: "COMMENT_RESPONSE_EMPTY",
    invalid: "COMMENT_RESPONSE_INVALID",
    failed: "COMMENT_COLLECT_FAILED",
};

const VIDEO_LIST_CODES: ResponseCodes = ResponseCodes {
    empty: "VIDEO_RESPONSE_EMPTY",
    invalid: "VIDEO_RESPONSE_INVALID",
    failed: "VIDEO_COLLECT_FAILED",
};

const VOLATILE_QUERY_KEYS: &[&str] = &["msToken", "a_bogus"];
const REPLY_INCOMPATIBLE_QUERY_KEYS: &[&str] =
    &["aweme_id", "insert_ids", "whale_cut_token", "rcFT"];

fn url_with_query(raw: &str, remove: &[&str], changes: &[(&str, String)]) -> String {
    let Ok(mut parsed) = Url::parse(raw) else {
        return raw.to_owned();
    };
    let mut pairs: BTreeMap<String, String> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    for key in VOLATILE_QUERY_KEYS.iter().chain(remove.iter()) {
        pairs.remove(*key);
    }
    for (k, v) in changes {
        pairs.insert((*k).to_owned(), v.clone());
    }
    {
        let mut ser = parsed.query_pairs_mut();
        ser.clear();
        for (k, v) in &pairs {
            ser.append_pair(k, v);
        }
    }
    parsed.to_string()
}

fn url_matches_aweme(raw: &str, aweme_id: &str) -> bool {
    let Ok(parsed) = Url::parse(raw) else {
        return false;
    };
    parsed
        .query_pairs()
        .any(|(k, v)| (k == "aweme_id" || k == "item_id") && v == aweme_id)
}

fn reply_url(
    root_template: &str,
    aweme_id: &str,
    root_cid: &str,
    cursor: &str,
    reported_reply_count: u64,
) -> String {
    let count = reported_reply_count.clamp(1, 10).to_string();
    let updated = url_with_query(
        root_template,
        REPLY_INCOMPATIBLE_QUERY_KEYS,
        &[
            ("cursor", cursor.to_owned()),
            ("item_id", aweme_id.to_owned()),
            ("comment_id", root_cid.to_owned()),
            ("count", count),
            ("item_type", "0".to_owned()),
            ("pc_img_format", "webp".to_owned()),
        ],
    );
    let Ok(mut parsed) = Url::parse(&updated) else {
        return updated;
    };
    let reply_path = web_path("comment_reply").unwrap_or("/aweme/v1/web/comment/list/reply/");
    parsed.set_path(reply_path);
    parsed.to_string()
}

fn exchange_to_proto_response(ex: &CapturedNetworkExchange) -> ProtoFetchResponse {
    let text = ex
        .response_body
        .as_deref()
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();
    ProtoFetchResponse {
        http_status: ex.status.unwrap_or(0),
        content_type: ex
            .response_headers
            .get("content-type")
            .cloned()
            .unwrap_or_default(),
        retry_after: ex.response_headers.get("retry-after").cloned(),
        text,
    }
}

fn value_as_string(val: Option<&Value>) -> String {
    match val {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn value_as_bool(val: Option<&Value>) -> bool {
    match val {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        Some(Value::String(s)) => matches!(s.as_str(), "1" | "true" | "True"),
        _ => false,
    }
}

const IDENTITY_CAPTURE_PATTERNS: &[&str] = &[
    "/aweme/v1/web/",
    "discover/search",
    "user/profile",
    "user/detail",
    "aweme/post",
];

fn is_html_document_exchange(ex: &CapturedNetworkExchange) -> bool {
    if ex
        .resource_type
        .as_deref()
        .is_some_and(|rt| rt.eq_ignore_ascii_case("Document"))
    {
        return true;
    }
    if ex
        .mime_type
        .as_deref()
        .is_some_and(|mime| mime.to_ascii_lowercase().contains("text/html"))
    {
        return true;
    }
    ex.response_headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("content-type") && v.to_ascii_lowercase().contains("text/html")
    })
}

fn is_ready_identity_api_exchange(ex: &CapturedNetworkExchange) -> bool {
    if ex.method.eq_ignore_ascii_case("OPTIONS")
        || ex.status.is_none()
        || ex.response_body.is_none()
        || is_html_document_exchange(ex)
    {
        return false;
    }
    let path = ex.url.split('?').next().unwrap_or(&ex.url).to_lowercase();
    IDENTITY_CAPTURE_PATTERNS.iter().any(|p| path.contains(p))
}

fn extract_identity_candidates(captured_values: &[Value]) -> Vec<Value> {
    let mut candidates: BTreeMap<(String, String, String), Value> = BTreeMap::new();
    for value in captured_values {
        for item in identities_in(value) {
            let k = (
                value_as_string(item.get("unique_id")),
                value_as_string(item.get("sec_uid")),
                value_as_string(item.get("uid")),
            );
            candidates.insert(k, item);
        }
    }
    candidates.into_values().collect()
}

fn is_search_verify_check(value: &Map<String, Value>) -> bool {
    value
        .get("search_nil_info")
        .and_then(Value::as_object)
        .and_then(|info| {
            info.get("search_nil_type")
                .or_else(|| info.get("search_nil_item"))
        })
        .and_then(Value::as_str)
        .is_some_and(|tag| tag.eq_ignore_ascii_case("verify_check"))
}

/// Resolve a Douyin handle (`unique_id`) to a structured user identity (`sec_uid`, `uid`, etc.).
pub async fn resolve_identity(
    session: &mut ChromeSession,
    home: &Path,
    data_root: &Path,
    profile: &str,
    douyin_id: &str,
) -> Result<Value> {
    let _ = session.ensure_authenticated(AuthPolicy::FailFast).await?;
    let encoded: String = url::form_urlencoded::byte_serialize(douyin_id.as_bytes()).collect();
    let needle = douyin_id.to_lowercase();
    let mut captured_values: Vec<Value> = Vec::new();
    let mut protocol_errors: Vec<DoudouError> = Vec::new();

    // Fast path: Douyin's in-page search autocomplete API (`/aweme/v1/web/search/sug/`)
    // returns exact `rich_sug_sec_uid` / `rich_sug_user_id` / `rich_sug_short_id` directly
    // without leaving `/chat` or triggering full-page search `verify_check`.
    if session.mode() != doudou_cdp::SessionMode::Fake {
        let sug_url = format!(
            "https://www.douyin.com/aweme/v1/web/search/sug/?keyword={encoded}&source=normal_search&aid=6383"
        );
        if let Ok(sug_map) = fetch_json(
            session,
            &sug_url,
            ResponseCodes {
                empty: "IDENTITY_RESPONSE_EMPTY",
                invalid: "IDENTITY_RESPONSE_INVALID",
                failed: "IDENTITY_LOOKUP_FAILED",
            },
            "identity.sug",
            15_000,
            false,
        )
        .await
        {
            captured_values.push(Value::Object(sug_map));
        }
    }

    let sug_candidates = extract_identity_candidates(&captured_values);
    let sug_exact_matches: Vec<&Value> = sug_candidates
        .iter()
        .filter(|item| {
            value_as_string(item.get("unique_id")).to_lowercase() == needle
                && !value_as_string(item.get("sec_uid")).is_empty()
        })
        .collect();

    let mut verify_check_seen = false;
    let values = if sug_exact_matches.len() == 1 {
        sug_candidates
    } else {
        let page = session.ensure_page().await?;
        let buffer = page
            .enable_network_capture(
                IDENTITY_CAPTURE_PATTERNS
                    .iter()
                    .map(|p| (*p).to_owned())
                    .collect(),
            )
            .await?;
        let mut seen_request_ids: HashSet<String> = buffer
            .snapshot()
            .into_iter()
            .map(|ex| ex.request_id)
            .collect();

        let search_url = format!("https://www.douyin.com/search/{encoded}?type=user");
        let _ = session.navigate_douyin(&search_url).await?;

        let ui_page = CdpUiPage::new(page);
        if let Ok(Some(render_text)) = VideoDetailComponent::new(&ui_page).render_data().await
            && let Ok(parsed) = parse_render_data(&render_text)
        {
            captured_values.push(Value::Object(parsed));
        }

        // Wait boundedly for JSON API search/profile responses with completed response bodies
        // (skipping HTML document navigations), checking both SSR #RENDER_DATA and XHR responses.
        let wait_budget = if session.mode() == doudou_cdp::SessionMode::Fake {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(10)
        };
        let deadline = Instant::now() + wait_budget;

        loop {
            let ready: Vec<CapturedNetworkExchange> = buffer
                .snapshot()
                .into_iter()
                .filter(|ex| {
                    !seen_request_ids.contains(&ex.request_id) && is_ready_identity_api_exchange(ex)
                })
                .collect();

            for ex in ready {
                seen_request_ids.insert(ex.request_id.clone());
                let proto = exchange_to_proto_response(&ex);
                match parse_result(
                    session,
                    &proto,
                    ResponseCodes {
                        empty: "IDENTITY_RESPONSE_EMPTY",
                        invalid: "IDENTITY_RESPONSE_INVALID",
                        failed: "IDENTITY_LOOKUP_FAILED",
                    },
                    "identity.lookup",
                )
                .await
                {
                    Ok(map) => {
                        if is_search_verify_check(&map) {
                            verify_check_seen = true;
                        }
                        captured_values.push(Value::Object(map));
                    }
                    Err(err) => protocol_errors.push(err),
                }
            }

            let current_values = extract_identity_candidates(&captured_values);
            let has_exact = current_values.iter().any(|item| {
                value_as_string(item.get("unique_id")).to_lowercase() == needle
                    && !value_as_string(item.get("sec_uid")).is_empty()
            });
            if has_exact {
                break current_values;
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break current_values;
            }
            let _ = buffer
                .wait_for(remaining.min(Duration::from_millis(250)), |ex| {
                    !seen_request_ids.contains(&ex.request_id) && is_ready_identity_api_exchange(ex)
                })
                .await;
        }
    };

    if values.is_empty() && verify_check_seen {
        return Err(DoudouError::new(
            "VERIFICATION_REQUIRED",
            "Douyin search requested account verification (`verify_check`).",
        )
        .with_details(json!({
            "douyin_id": douyin_id,
            "operation": "identity.lookup",
            "search_nil_type": "verify_check",
        })));
    }

    let exact: Vec<&Value> = values
        .iter()
        .filter(|item| {
            value_as_string(item.get("unique_id")).to_lowercase() == needle
                && !value_as_string(item.get("sec_uid")).is_empty()
        })
        .collect();

    if exact.len() != 1 {
        if let Some(blocking) = protocol_errors.iter().find(|e| {
            matches!(
                e.code.as_str(),
                "LOGIN_REQUIRED" | "VERIFICATION_REQUIRED" | "RATE_LIMITED"
            )
        }) {
            return Err(blocking.clone());
        }
        let code = if exact.len() > 1 {
            "IDENTITY_AMBIGUOUS"
        } else {
            "IDENTITY_NOT_FOUND"
        };
        let err_codes: Vec<String> = protocol_errors
            .iter()
            .map(|e| e.code.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        return Err(DoudouError::new(
            code,
            "The Douyin ID did not resolve to one exact structured identity.",
        )
        .with_details(json!({
            "douyin_id": douyin_id,
            "candidates": values,
            "protocol_error_codes": err_codes,
        })));
    }

    let mut cache = IdentityCache::load(home, data_root, profile)?;
    let merged_values: Vec<Value> = values
        .iter()
        .map(|item| cache.merge(item, "identity-resolve"))
        .collect();
    let exact_identity = merged_values
        .iter()
        .find(|item| value_as_string(item.get("unique_id")).to_lowercase() == needle)
        .cloned()
        .expect("exact identity validated above");
    cache.save()?;

    let files = write_identity(home, data_root, profile, &exact_identity, &merged_values)?;
    Ok(json!({
        "status": "resolved",
        "identity": exact_identity,
        "candidates": merged_values,
        "files": files,
    }))
}

async fn next_video_list_page(
    session: &mut ChromeSession,
    buffer: &doudou_cdp::NetworkCaptureBuffer,
    seen_request_ids: &mut HashSet<String>,
    timeout: Duration,
    ignored_non_json: &mut usize,
) -> Result<(String, Map<String, Value>)> {
    let list_path = web_path("video_list").unwrap_or("/aweme/v1/web/aweme/post/");
    let deadline = Instant::now() + timeout;
    let mut last_invalid: Option<DoudouError> = None;

    loop {
        let ready: Vec<CapturedNetworkExchange> = buffer
            .snapshot()
            .into_iter()
            .filter(|ex| {
                !seen_request_ids.contains(&ex.request_id)
                    && ex.url.contains(list_path)
                    && ex.status.is_some()
                    && ex.response_body.is_some()
            })
            .collect();

        for ex in ready {
            seen_request_ids.insert(ex.request_id.clone());
            let proto = exchange_to_proto_response(&ex);
            match parse_result(session, &proto, VIDEO_LIST_CODES, "video.list").await {
                Ok(parsed) => return Ok((ex.url, parsed)),
                Err(err) => {
                    if err.code == "VIDEO_RESPONSE_INVALID"
                        && proto.content_type.to_lowercase().contains("html")
                    {
                        *ignored_non_json += 1;
                        last_invalid = Some(err);
                        continue;
                    }
                    return Err(err);
                }
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            if let Some(err) = last_invalid {
                return Err(err);
            }
            return Err(DoudouError::new(
                "PROTOCOL_TRANSPORT_TIMEOUT",
                "The profile page did not emit a video-list protocol response.",
            )
            .with_details(json!({
                "operation": "video.list",
                "timeout_seconds": timeout.as_secs_f64(),
            })));
        }
        let _ = buffer
            .wait_for(remaining.min(Duration::from_millis(50)), |ex| {
                !seen_request_ids.contains(&ex.request_id)
                    && ex.url.contains(list_path)
                    && ex.status.is_some()
                    && ex.response_body.is_some()
            })
            .await;
    }
}

/// Collect a Douyin user's published videos by navigating to their profile and scrolling.
pub async fn collect_user_videos(
    session: &mut ChromeSession,
    home: &Path,
    data_root: &Path,
    profile: &str,
    identity: &Value,
    max_pages: usize,
) -> Result<Value> {
    let _ = session.ensure_authenticated(AuthPolicy::FailFast).await?;
    let sec_uid = identity
        .get("sec_uid")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| DoudouError::new("INVALID_IDENTITY", "User identity is missing sec_uid."))?;
    let list_path = web_path("video_list").unwrap_or("/aweme/v1/web/aweme/post/");
    let page = session.ensure_page().await?;
    let buffer = page
        .enable_network_capture(vec![list_path.to_owned()])
        .await?;
    let mut seen_request_ids: HashSet<String> = buffer
        .snapshot()
        .into_iter()
        .map(|ex| ex.request_id)
        .collect();

    let profile_url = format!("https://www.douyin.com/user/{sec_uid}?from_tab_name=main");
    let _ = session.navigate_douyin(&profile_url).await?;

    let mut ignored_non_json = 0usize;
    let initial_timeout = if session.mode() == doudou_cdp::SessionMode::Fake {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(30)
    };
    let (_template, mut value) = next_video_list_page(
        session,
        &buffer,
        &mut seen_request_ids,
        initial_timeout,
        &mut ignored_non_json,
    )
    .await?;

    let ui_page = CdpUiPage::new(page);
    let video_list = ProfileVideoListComponent::new(&ui_page);
    let _ = video_list.scroll_container().await?;

    let mut videos: HashMap<String, Value> = HashMap::new();
    let mut pages: Vec<Value> = Vec::new();
    let mut cursor_in = json!(0);
    let mut seen_cursors: HashSet<String> = HashSet::new();
    let mut finished = false;

    for page_number in 1..=max_pages {
        let raw_items = contract_list(&value, "video_items", "video.list")?;
        let mut added = 0usize;
        for raw in raw_items {
            let item = normalize_video(raw);
            let aid = value_as_string(item.get("aweme_id"));
            if !aid.is_empty() && !videos.contains_key(&aid) {
                videos.insert(aid, item);
                added += 1;
            }
        }
        let cursor_out = contract_value(&value, "video_cursor").cloned();
        let has_more = value_as_bool(contract_value(&value, "has_more"));
        pages.push(json!({
            "page": page_number,
            "cursor_in": cursor_in,
            "cursor_out": cursor_out,
            "received": raw_items.len(),
            "added": added,
            "has_more": has_more,
        }));
        if !has_more {
            finished = true;
            break;
        }
        let cursor_out_str = cursor_out
            .as_ref()
            .map(|v| value_as_string(Some(v)))
            .unwrap_or_default();
        if cursor_out_str.is_empty() || !seen_cursors.insert(cursor_out_str) {
            return Err(DoudouError::new(
                "VIDEO_CURSOR_STALLED",
                "Video pagination cursor did not advance.",
            )
            .with_details(json!({ "cursor": cursor_out })));
        }
        cursor_in = cursor_out.unwrap_or(Value::Null);

        let mut next_loaded = false;
        for attempt in 1..=5 {
            let _ = video_list.scroll_to_end().await?;
            match next_video_list_page(
                session,
                &buffer,
                &mut seen_request_ids,
                Duration::from_secs(4),
                &mut ignored_non_json,
            )
            .await
            {
                Ok((_, next_val)) => {
                    value = next_val;
                    next_loaded = true;
                    break;
                }
                Err(err) if err.code == "PROTOCOL_TRANSPORT_TIMEOUT" && attempt < 5 => {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                Err(err) => return Err(err),
            }
        }
        if !next_loaded {
            return Err(DoudouError::new(
                "PROTOCOL_TRANSPORT_TIMEOUT",
                "The profile page did not emit the next video-list page.",
            ));
        }
    }

    if !finished {
        return Err(DoudouError::new(
            "VIDEO_PAGE_LIMIT",
            "Video collection reached the configured page limit.",
        )
        .with_details(json!({ "max_pages": max_pages })));
    }

    let mut cache = IdentityCache::load(home, data_root, profile)?;
    let mut ordered: Vec<Value> = videos
        .into_values()
        .map(|item| enrich_known_identities(&item, &cache, "video-list"))
        .collect();
    for item in &ordered {
        if let Some(author) = item.get("author") {
            let _ = cache.merge(author, "video-list");
        }
    }
    cache.save()?;

    ordered.sort_by(|left, right| {
        let key = |v: &Value| {
            let ct = v.get("create_time").and_then(Value::as_i64).unwrap_or(0);
            let aid = value_as_string(v.get("aweme_id"));
            (std::cmp::Reverse(ct), aid)
        };
        key(left).cmp(&key(right))
    });

    let files = write_video_index(home, data_root, profile, identity, &ordered, &pages)?;
    Ok(json!({
        "status": "collected",
        "identity": identity,
        "count": ordered.len(),
        "pages": pages,
        "pagination_trigger": "profile_component_scroll",
        "ignored_non_json_responses": ignored_non_json,
        "files": files,
    }))
}

async fn collect_reply_thread(
    session: &mut ChromeSession,
    reply_template: &str,
    aweme_id: &str,
    root_cid: &str,
    reported_reply_count: u64,
    max_pages: usize,
) -> Result<(Vec<Value>, Vec<Value>)> {
    let mut cursor = "0".to_owned();
    let mut pages = Vec::new();
    let mut comments = Vec::new();
    let pacing = if session.mode() == doudou_cdp::SessionMode::Fake {
        Duration::from_millis(1)
    } else {
        Duration::from_secs(3)
    };

    for page_number in 1..=max_pages {
        tokio::time::sleep(pacing).await;
        let url = reply_url(
            reply_template,
            aweme_id,
            root_cid,
            &cursor,
            reported_reply_count,
        );
        let value = match fetch_json(
            session,
            &url,
            COMMENT_CODES,
            "comment.reply.list",
            15_000,
            true,
        )
        .await
        {
            Ok(v) => v,
            Err(mut err) => {
                let parsed_url = Url::parse(&url).ok();
                let query_map: BTreeMap<String, String> = parsed_url
                    .as_ref()
                    .map(|u| {
                        u.query_pairs()
                            .map(|(k, v)| (k.into_owned(), v.into_owned()))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut details = err
                    .details
                    .as_ref()
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                details.insert(
                    "request_contract".to_owned(),
                    json!({
                        "path": parsed_url.as_ref().map_or("", Url::path),
                        "query_keys": query_map.keys().cloned().collect::<Vec<_>>(),
                        "item_id_matches": query_map.get("item_id").map(String::as_str) == Some(aweme_id),
                        "count": query_map.get("count"),
                        "cursor": query_map.get("cursor"),
                    }),
                );
                err.details = Some(Value::Object(details));
                return Err(err);
            }
        };

        let batch = contract_list(&value, "comments", "comment.reply.list")?;
        let next_cursor_val = contract_value(&value, "comment_cursor").cloned();
        let next_cursor = value_as_string(next_cursor_val.as_ref());
        let has_more = value_as_bool(contract_value(&value, "has_more"));
        for raw in batch {
            comments.push(normalize_comment(raw, aweme_id));
        }
        pages.push(json!({
            "page": page_number,
            "cursor_in": cursor,
            "cursor_out": next_cursor_val,
            "received": batch.len(),
            "has_more": has_more,
        }));

        if !has_more {
            return Ok((pages, comments));
        }
        if next_cursor == cursor {
            return Err(DoudouError::new(
                "COMMENT_CURSOR_STALLED",
                "Reply comment cursor did not advance.",
            )
            .with_details(json!({ "root_cid": root_cid })));
        }
        cursor = next_cursor;
    }

    Err(DoudouError::new(
        "COMMENT_PAGE_LIMIT",
        "Reply comment collection reached the configured page limit.",
    )
    .with_details(json!({ "root_cid": root_cid, "max_pages": max_pages })))
}

async fn download_cover_bytes(cover_url: &str) -> Option<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .ok()?;
    let resp = client
        .get(cover_url)
        .header("referer", "https://www.douyin.com/")
        .send()
        .await
        .ok()?;
    if resp.status() != reqwest::StatusCode::OK {
        return None;
    }
    let bytes = resp.bytes().await.ok()?.to_vec();
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"\x89PNG")
        || bytes.starts_with(b"RIFF")
    {
        Some(bytes)
    } else {
        None
    }
}

/// Collect a video's detail metadata, cover image, and full root + reply comments with
/// resumable on-disk pagination checkpoints.
pub async fn collect_video(
    session: &mut ChromeSession,
    home: &Path,
    data_root: &Path,
    profile: &str,
    _sec_uid: &str,
    aweme_id: &str,
    max_pages: usize,
) -> Result<Value> {
    let _ = session.ensure_authenticated(AuthPolicy::FailFast).await?;
    let root_path = web_path("comment_root").unwrap_or("/aweme/v1/web/comment/list/");
    let reply_path = web_path("comment_reply").unwrap_or("/aweme/v1/web/comment/list/reply/");
    let detail_path = web_path("video_detail").unwrap_or("/aweme/v1/web/aweme/detail/");

    let page = session.ensure_page().await?;
    let buffer = page
        .enable_network_capture(vec![
            root_path.to_owned(),
            reply_path.to_owned(),
            detail_path.to_owned(),
        ])
        .await?;
    let mut seen_request_ids: HashSet<String> = buffer
        .snapshot()
        .into_iter()
        .map(|ex| ex.request_id)
        .collect();

    let source_url = format!("https://www.douyin.com/video/{aweme_id}");
    let _ = session.navigate_douyin(&source_url).await?;

    let ui_page = CdpUiPage::new(page);
    let mut render_data = Value::Object(Map::new());
    if let Ok(Some(render_text)) = VideoDetailComponent::new(&ui_page).render_data().await
        && let Ok(parsed) = parse_render_data(&render_text)
    {
        render_data = Value::Object(parsed);
    }

    let app_data = render_data.get("app").cloned().unwrap_or_else(|| json!({}));
    let mut detail = app_data
        .get("videoDetail")
        .cloned()
        .unwrap_or_else(|| json!({}));

    if detail.as_object().is_none_or(Map::is_empty) {
        let wait = if session.mode() == doudou_cdp::SessionMode::Fake {
            Duration::from_secs(2)
        } else {
            Duration::from_secs(20)
        };
        let found = buffer
            .wait_for(wait, |ex| {
                !seen_request_ids.contains(&ex.request_id)
                    && ex.url.contains(detail_path)
                    && ex.status.is_some()
                    && ex.response_body.is_some()
            })
            .await
            .ok_or_else(|| {
                DoudouError::new(
                    "PROTOCOL_TRANSPORT_TIMEOUT",
                    "Timed out waiting for video detail protocol response.",
                )
            })?;
        seen_request_ids.insert(found.request_id.clone());
        let proto = exchange_to_proto_response(&found);
        let detail_value = parse_result(
            session,
            &proto,
            ResponseCodes {
                empty: "VIDEO_DETAIL_RESPONSE_EMPTY",
                invalid: "VIDEO_DETAIL_RESPONSE_INVALID",
                failed: "VIDEO_DETAIL_FAILED",
            },
            "video.detail",
        )
        .await?;
        detail = contract_value(&detail_value, "video_detail")
            .cloned()
            .unwrap_or_else(|| json!({}));
    }

    let mut normalized = normalize_video_detail(&detail);
    let actual_aweme = value_as_string(normalized.get("aweme_id"));
    if actual_aweme != aweme_id {
        let cur_url = ui_page.url().await.unwrap_or_default();
        let page_url = cur_url.split('?').next().unwrap_or(&cur_url);
        let mut render_keys: Vec<String> = render_data
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        render_keys.sort();
        let mut app_keys: Vec<String> = app_data
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        app_keys.sort();
        return Err(DoudouError::new(
            "VIDEO_DETAIL_MISMATCH",
            "SSR detail does not match the requested video.",
        )
        .with_details(json!({
            "expected": aweme_id,
            "actual": actual_aweme,
            "page_url": page_url,
            "render_keys": render_keys,
            "app_keys": app_keys,
        })));
    }

    if session.mode() != doudou_cdp::SessionMode::Fake {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    let _ = VideoCommentsPanelComponent.ensure_open(&ui_page).await?;
    if session.mode() != doudou_cdp::SessionMode::Fake {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let find_root_template = |buf: &doudou_cdp::NetworkCaptureBuffer,
                              seen: &HashSet<String>|
     -> Option<(String, String)> {
        let snap = buf.snapshot();
        snap.into_iter()
            .rev()
            .find(|ex| {
                !seen.contains(&ex.request_id)
                    && ex.url.contains(root_path)
                    && !ex.url.contains(reply_path)
                    && url_matches_aweme(&ex.url, aweme_id)
                    && ex.status.is_some()
                    && ex.response_body.is_some()
            })
            .map(|ex| (ex.request_id, ex.url))
    };

    let root_template = if let Some((req_id, url)) = find_root_template(&buffer, &seen_request_ids)
    {
        seen_request_ids.insert(req_id);
        url
    } else {
        let wait = if session.mode() == doudou_cdp::SessionMode::Fake {
            Duration::from_secs(2)
        } else {
            Duration::from_secs(20)
        };
        let matched = buffer
            .wait_for(wait, |ex| {
                !seen_request_ids.contains(&ex.request_id)
                    && ex.url.contains(root_path)
                    && !ex.url.contains(reply_path)
                    && url_matches_aweme(&ex.url, aweme_id)
                    && ex.status.is_some()
                    && ex.response_body.is_some()
            })
            .await
            .ok_or_else(|| {
                DoudouError::new(
                    "PROTOCOL_TRANSPORT_TIMEOUT",
                    "Timed out waiting for comment root template URL.",
                )
            })?;
        seen_request_ids.insert(matched.request_id);
        matched.url
    };

    let (cp_opt, partial) = read_comment_checkpoint(home, data_root, profile, aweme_id)?;
    let (checkpoint, mut comments) = match cp_opt {
        Some(cp) if cp.get("status").and_then(Value::as_str) != Some("completed") => (cp, partial),
        _ => (json!({}), Vec::new()),
    };

    let mut pages: Vec<Value> = checkpoint
        .get("pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut embedded_reply_counts: BTreeMap<String, usize> = checkpoint
        .get("embedded_reply_counts")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_u64().unwrap_or(0) as usize))
                .collect()
        })
        .unwrap_or_default();
    let mut completed_reply_root_ids: BTreeSet<String> = checkpoint
        .get("completed_reply_root_ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let mut completed_root_pages = checkpoint
        .get("completed_root_pages")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let mut root_complete = checkpoint
        .get("root_complete")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut cursor = checkpoint
        .get("root_cursor")
        .map(|v| value_as_string(Some(v)))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "0".to_owned());

    let persist = |status: &str,
                   cursor: &str,
                   root_complete: bool,
                   completed_root_pages: usize,
                   completed_reply_root_ids: &BTreeSet<String>,
                   next_reply_root_id: Option<&str>,
                   embedded_reply_counts: &BTreeMap<String, usize>,
                   pages: &[Value],
                   error: Option<&DoudouError>,
                   records: &[Value]|
     -> Result<()> {
        let mut record = Map::new();
        record.insert("status".to_owned(), json!(status));
        record.insert("root_cursor".to_owned(), json!(cursor));
        record.insert("root_complete".to_owned(), json!(root_complete));
        record.insert(
            "completed_root_pages".to_owned(),
            json!(completed_root_pages),
        );
        record.insert(
            "completed_reply_root_ids".to_owned(),
            json!(completed_reply_root_ids.iter().collect::<Vec<_>>()),
        );
        record.insert("next_reply_root_id".to_owned(), json!(next_reply_root_id));
        record.insert(
            "embedded_reply_counts".to_owned(),
            json!(embedded_reply_counts),
        );
        record.insert("pages".to_owned(), Value::Array(pages.to_vec()));
        if let Some(err) = error {
            record.insert(
                "last_error".to_owned(),
                json!({ "code": err.code, "details": err.details }),
            );
        }
        let _ = write_comment_checkpoint(
            home,
            data_root,
            profile,
            aweme_id,
            &Value::Object(record),
            records,
        )?;
        Ok(())
    };

    let paginate_outcome: Result<()> = async {
        if !root_complete {
            let mut finished_roots = false;
            for page_number in (completed_root_pages + 1)..=max_pages {
                let page_url = url_with_query(&root_template, &[], &[("cursor", cursor.clone())]);
                let value = fetch_json(
                    session,
                    &page_url,
                    COMMENT_CODES,
                    "comment.root.list",
                    15_000,
                    true,
                )
                .await?;
                let raw_roots = contract_list(&value, "comments", "comment.root.list")?;
                let batch: Vec<Value> = raw_roots
                    .iter()
                    .map(|raw| normalize_comment(raw, aweme_id))
                    .collect();
                for (raw_root, root) in raw_roots.iter().zip(batch.iter()) {
                    comments.push(root.clone());
                    let embedded = raw_root
                        .as_object()
                        .and_then(|obj| contract_value(obj, "embedded_replies"))
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let root_cid = value_as_string(root.get("cid"));
                    embedded_reply_counts.insert(root_cid, embedded.len());
                    for emb in &embedded {
                        comments.push(normalize_comment(emb, aweme_id));
                    }
                }
                let next_cursor_val = contract_value(&value, "comment_cursor").cloned();
                let next_cursor = value_as_string(next_cursor_val.as_ref());
                let has_more = value_as_bool(contract_value(&value, "has_more"));
                pages.push(json!({
                    "kind": "root",
                    "page": page_number,
                    "cursor_in": cursor,
                    "cursor_out": next_cursor_val,
                    "received": batch.len(),
                    "has_more": has_more,
                }));
                completed_root_pages = page_number;
                root_complete = !has_more;
                if has_more && next_cursor == cursor {
                    return Err(DoudouError::new(
                        "COMMENT_CURSOR_STALLED",
                        "Root comment cursor did not advance.",
                    ));
                }
                cursor = next_cursor;
                persist(
                    "running",
                    &cursor,
                    root_complete,
                    completed_root_pages,
                    &completed_reply_root_ids,
                    None,
                    &embedded_reply_counts,
                    &pages,
                    None,
                    &comments,
                )?;
                if root_complete {
                    finished_roots = true;
                    break;
                }
            }
            if !finished_roots {
                return Err(DoudouError::new(
                    "COMMENT_PAGE_LIMIT",
                    "Root comment collection reached the configured page limit.",
                )
                .with_details(json!({ "max_pages": max_pages })));
            }
        }

        let reply_targets: Vec<(String, u64)> = comments
            .iter()
            .filter_map(|item| {
                let cid = value_as_string(item.get("cid"));
                let reply_id = value_as_string(item.get("reply_id"));
                if cid.is_empty() || cid != reply_id {
                    return None;
                }
                let reported = item
                    .get("reported_reply_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let embedded = embedded_reply_counts.get(&cid).copied().unwrap_or(0) as u64;
                if reported > embedded && !completed_reply_root_ids.contains(&cid) {
                    Some((cid, reported))
                } else {
                    None
                }
            })
            .collect();

        let reply_template = if reply_targets.is_empty() {
            root_template.clone()
        } else {
            let triggered = match VideoCommentsPanelComponent
                .trigger_reply_template(&ui_page, 2)
                .await
            {
                Ok(n) => n,
                Err(err) if err.code == "PAGE_ADAPTER_DRIFTED" => 1,
                Err(err) => return Err(err),
            };
            if triggered == 0 {
                root_template.clone()
            } else {
                let wait = if session.mode() == doudou_cdp::SessionMode::Fake {
                    Duration::from_millis(200)
                } else {
                    Duration::from_secs(20)
                };
                let matched_reply = buffer
                    .wait_for(wait, |ex| {
                        !seen_request_ids.contains(&ex.request_id)
                            && ex.url.contains(reply_path)
                            && url_matches_aweme(&ex.url, aweme_id)
                            && ex.status.is_some()
                            && ex.response_body.is_some()
                    })
                    .await;
                if let Some(ex) = matched_reply {
                    seen_request_ids.insert(ex.request_id);
                    ex.url
                } else {
                    root_template.clone()
                }
            }
        };

        for (root_cid, reported) in &reply_targets {
            let (reply_pages, reply_comments) = match collect_reply_thread(
                session,
                &reply_template,
                aweme_id,
                root_cid,
                *reported,
                max_pages,
            )
            .await
            {
                Ok(res) => res,
                Err(mut err) => {
                    let mut details = err
                        .details
                        .as_ref()
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    details.insert("root_cid".to_owned(), json!(root_cid));
                    err.details = Some(Value::Object(details));
                    return Err(err);
                }
            };
            comments.extend(reply_comments);
            for mut rp in reply_pages {
                if let Some(obj) = rp.as_object_mut() {
                    obj.insert("kind".to_owned(), json!("reply"));
                    obj.insert("root_cid".to_owned(), json!(root_cid));
                }
                pages.push(rp);
            }
            completed_reply_root_ids.insert(root_cid.clone());
            let next_pending = reply_targets
                .iter()
                .find(|(cid, _)| !completed_reply_root_ids.contains(cid))
                .map(|(cid, _)| cid.as_str());
            persist(
                "running",
                &cursor,
                root_complete,
                completed_root_pages,
                &completed_reply_root_ids,
                next_pending,
                &embedded_reply_counts,
                &pages,
                None,
                &comments,
            )?;
        }
        Ok(())
    }
    .await;

    if let Err(err) = paginate_outcome {
        let _ = persist(
            "failed",
            &cursor,
            root_complete,
            completed_root_pages,
            &completed_reply_root_ids,
            None,
            &embedded_reply_counts,
            &pages,
            Some(&err),
            &comments,
        );
        return Err(err);
    }

    let current_comments = merge_video_comments(&[], &comments);
    let cover = match normalized
        .get("cover_url")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        Some(url) if session.mode() != doudou_cdp::SessionMode::Fake => {
            download_cover_bytes(url).await
        }
        _ => None,
    };

    let mut cache = IdentityCache::load(home, data_root, profile)?;
    normalized = enrich_known_identities(&normalized, &cache, "video-detail");
    let enriched_current: Vec<Value> = current_comments
        .iter()
        .map(|item| enrich_known_identities(item, &cache, "comment"))
        .collect();
    let existing = read_video_comments(home, data_root, profile, aweme_id)?;
    let mut merged_comments = merge_video_comments(&existing, &enriched_current);
    resolve_comment_parents(&mut merged_comments);
    if let Some(author) = normalized.get("author") {
        let _ = cache.merge(author, "video-detail");
    }
    for comment in &merged_comments {
        if let Some(user) = comment.get("user") {
            let _ = cache.merge(user, "comment");
        }
    }
    cache.save()?;

    let files = write_video(
        home,
        data_root,
        profile,
        &normalized,
        cover.as_deref(),
        &merged_comments,
        &pages,
    )?;
    persist(
        "completed",
        &cursor,
        root_complete,
        completed_root_pages,
        &completed_reply_root_ids,
        None,
        &embedded_reply_counts,
        &pages,
        None,
        &enriched_current,
    )?;

    Ok(json!({
        "status": "collected",
        "video": normalized,
        "comments": merged_comments.len(),
        "pages": pages,
        "files": files,
    }))
}

#[cfg(test)]
mod tests {
    use doudou_cdp::{CdpPageOps as _, FakeBrowserSession};
    use doudou_core::profile_paths;
    use doudou_storage::create_profile;
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn resolve_identity_skips_html_document_and_waits_for_response_body() {
        let dir = TempDir::new().expect("tempdir");
        let home = dir.path().join("home");
        let root = dir.path().join("project");
        create_profile(&home, &root, "work", None).expect("profile");
        let paths = profile_paths(&home, &root, "work").expect("paths");

        let mut fake = FakeBrowserSession::authenticated(paths);
        let page = std::sync::Arc::clone(fake.fake_page());
        let buffer = page.capture_buffer();

        // Simulate live navigation order:
        // 1. HTML Document response arrives first (must be skipped, never treated as verification).
        // 2. JSON search XHR arrives with status=Some(200) and response_body=None (in-flight).
        // 3. Network.getResponseBody populates response_body shortly afterward.
        let producer = tokio::spawn(async move {
            for _ in 0..100 {
                if !page.navigations().is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }

            let mut html_headers = BTreeMap::new();
            html_headers.insert(
                "content-type".to_owned(),
                "text/html; charset=utf-8".to_owned(),
            );
            buffer.upsert(CapturedNetworkExchange {
                request_id: "doc-html-1".to_owned(),
                url: "https://www.douyin.com/search/Rurutia.wow?type=user".to_owned(),
                method: "GET".to_owned(),
                status: Some(200),
                mime_type: Some("text/html".to_owned()),
                request_headers: BTreeMap::new(),
                response_headers: html_headers,
                request_body: None,
                response_body: Some(b"<html><body>captcha check</body></html>".to_vec()),
                resource_type: Some("Document".to_owned()),
                timestamp: doudou_core::utc_now(),
            });

            let mut json_headers = BTreeMap::new();
            json_headers.insert("content-type".to_owned(), "application/json".to_owned());
            buffer.upsert(CapturedNetworkExchange {
                request_id: "xhr-search-1".to_owned(),
                url: "https://www.douyin.com/aweme/v1/web/discover/search/?keyword=Rurutia.wow"
                    .to_owned(),
                method: "GET".to_owned(),
                status: Some(200),
                mime_type: Some("application/json".to_owned()),
                request_headers: BTreeMap::new(),
                response_headers: json_headers,
                request_body: None,
                response_body: None,
                resource_type: Some("XHR".to_owned()),
                timestamp: doudou_core::utc_now(),
            });

            tokio::time::sleep(Duration::from_millis(15)).await;
            let body = serde_json::to_vec(&json!({
                "status_code": 0,
                "user_list": [{
                    "user_info": {
                        "unique_id": "Rurutia.wow",
                        "sec_uid": "MS4wLjAB_rurutia",
                        "uid": "123456",
                        "nickname": "Rurutia"
                    }
                }]
            }))
            .expect("serialize search json");
            buffer.update_by_id("xhr-search-1", |ex| {
                ex.response_body = Some(body);
            });
        });

        let result = resolve_identity(fake.session_mut(), &home, &root, "work", "Rurutia.wow")
            .await
            .expect("resolve_identity should skip HTML and resolve JSON XHR");
        producer.await.expect("producer task");

        assert_eq!(result["status"], "resolved");
        assert_eq!(result["identity"]["sec_uid"], "MS4wLjAB_rurutia");
        assert_eq!(result["identity"]["unique_id"], "Rurutia.wow");

        let patterns = fake.fake_page().network_capture_patterns();
        assert_eq!(patterns.len(), 1);
        assert!(
            !patterns[0].iter().any(|p| p == "search"),
            "capture patterns must not include bare 'search': {:?}",
            patterns[0]
        );
        assert!(patterns[0].iter().any(|p| p == "/aweme/v1/web/"));
    }

    #[tokio::test]
    async fn next_video_list_page_waits_for_inflight_response_body_without_skipping() {
        let dir = TempDir::new().expect("tempdir");
        let home = dir.path().join("home");
        let root = dir.path().join("project");
        create_profile(&home, &root, "work", None).expect("profile");
        let paths = profile_paths(&home, &root, "work").expect("paths");

        let mut fake = FakeBrowserSession::authenticated(paths);
        let buffer = fake.fake_page().capture_buffer();
        let mut seen = HashSet::new();
        let mut ignored_non_json = 0usize;

        let mut headers = BTreeMap::new();
        headers.insert("content-type".to_owned(), "application/json".to_owned());
        buffer.upsert(CapturedNetworkExchange {
            request_id: "post-1".to_owned(),
            url: "https://www.douyin.com/aweme/v1/web/aweme/post/?sec_user_id=sec1".to_owned(),
            method: "GET".to_owned(),
            status: Some(200),
            mime_type: Some("application/json".to_owned()),
            request_headers: BTreeMap::new(),
            response_headers: headers,
            request_body: None,
            response_body: None,
            resource_type: Some("XHR".to_owned()),
            timestamp: doudou_core::utc_now(),
        });

        let buf_clone = buffer.clone();
        let body_filler = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(15)).await;
            let payload = serde_json::to_vec(&json!({
                "status_code": 0,
                "aweme_list": [{ "aweme_id": "777", "desc": "hello" }],
                "max_cursor": 0,
                "has_more": 0
            }))
            .expect("serialize post json");
            buf_clone.update_by_id("post-1", |ex| {
                ex.response_body = Some(payload);
            });
        });

        let (url, parsed) = next_video_list_page(
            fake.session_mut(),
            &buffer,
            &mut seen,
            Duration::from_secs(2),
            &mut ignored_non_json,
        )
        .await
        .expect("should wait for response_body instead of skipping");
        body_filler.await.expect("body filler");

        assert!(url.contains("/aweme/v1/web/aweme/post/"));
        assert_eq!(parsed["aweme_list"][0]["aweme_id"], "777");
        assert!(seen.contains("post-1"));
        assert_eq!(ignored_non_json, 0);
    }
}
