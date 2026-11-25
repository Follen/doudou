//! Batch send spacing/fail-closed tests, stranger chat integration tests,
//! video interaction planning/verification tests, and CDP-to-UI bridge tests.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use doudou_cdp::{
    CapturedNetworkExchange, FakeBrowserSession, FakeCdpPage, FetchResponse as CdpFetchResponse,
};
use doudou_core::profile_paths;
use doudou_protocol::{
    bytes_field, decode_fields, encode_fields, get_field, stranger_messages_request,
    stranger_request, string_field, varint_field,
};
use doudou_runtime::cdp_ui_bridge::{CdpUiPage, parse_key_spec};
use doudou_runtime::features::chat::{
    execute_batch_with, execute_like_with_readback, execute_send_with_readback,
    plan_batch_from_conversations, plan_like_for_conversation, plan_send_for_conversation,
    verify_send_readback,
};
use doudou_runtime::features::video::{
    execute_interaction_with, plan_comment, plan_comment_like, plan_reply,
    verify_interaction_readback,
};
use doudou_runtime::reads::chat::{
    STRANGER_CONVERSATIONS_ENDPOINT, STRANGER_MESSAGES_ENDPOINT, apply_conversation_identities,
    collect_messages, filter_conversations, refresh_session_index,
};
use doudou_storage::{
    DEFAULT_PLAN_TTL_SECONDS, PlanStatus, create_plan, create_profile, load_plan,
    read_conversation_index, write_conversation_index, write_video,
};
use doudou_ui::{CaptureFilter, MouseButton, SelectorQuery, UiPage};
use serde_json::{Value, json};
use tempfile::TempDir;

fn make_batch_plan(home: &std::path::Path, root: &std::path::Path, task_count: usize) -> String {
    let tasks: Vec<Value> = (0..task_count)
        .map(|index| {
            json!({
                "conversation": {
                    "conversation_id": (index + 1).to_string(),
                    "conversation_short_id": (index + 1).to_string(),
                    "conversation_type": 2,
                },
                "mode": "text",
                "text": "hello",
            })
        })
        .collect();
    create_plan(
        home,
        root,
        "work",
        "chat.send.batch",
        json!({ "tasks": tasks, "interval_seconds": 5.0 }),
        DEFAULT_PLAN_TTL_SECONDS,
    )
    .expect("create batch plan")
    .plan_id
}

#[tokio::test]
async fn batch_successful_attempts_are_spaced_by_at_least_five_seconds() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");

    let plan_id = make_batch_plan(&home, &root, 3);
    let clock = Arc::new(Mutex::new((0.0_f64, Vec::<f64>::new())));
    let attempts = Arc::new(Mutex::new(Vec::<f64>::new()));

    let clock_send = Arc::clone(&clock);
    let attempts_send = Arc::clone(&attempts);
    let clock_sleep = Arc::clone(&clock);

    let result = execute_batch_with(
        &home,
        &root,
        "work",
        &plan_id,
        move |_subplan| {
            let clock_send = Arc::clone(&clock_send);
            let attempts_send = Arc::clone(&attempts_send);
            async move {
                let now = clock_send.lock().expect("clock").0;
                let idx = {
                    let mut list = attempts_send.lock().expect("attempts");
                    list.push(now);
                    list.len()
                };
                Ok(json!({
                    "status": "sent",
                    "server_message_id": idx.to_string(),
                    "verification": { "passed": true },
                }))
            }
        },
        move |seconds| {
            let clock_sleep = Arc::clone(&clock_sleep);
            async move {
                let mut guard = clock_sleep.lock().expect("clock");
                guard.1.push(seconds);
                guard.0 += seconds;
            }
        },
    )
    .await
    .expect("batch should succeed");

    assert_eq!(result["status"], "sent");
    assert_eq!(*attempts.lock().expect("attempts"), vec![0.0, 5.0, 10.0]);
    assert_eq!(clock.lock().expect("clock").1, vec![5.0, 5.0]);
}

#[tokio::test]
async fn batch_first_failure_stops_and_marks_remaining_items_skipped() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");

    let plan_id = make_batch_plan(&home, &root, 3);
    let clock = Arc::new(Mutex::new((0.0_f64, Vec::<f64>::new())));
    let attempts = Arc::new(Mutex::new(Vec::<f64>::new()));

    let clock_send = Arc::clone(&clock);
    let attempts_send = Arc::clone(&attempts);
    let clock_sleep = Arc::clone(&clock);

    let result = execute_batch_with(
        &home,
        &root,
        "work",
        &plan_id,
        move |_subplan| {
            let clock_send = Arc::clone(&clock_send);
            let attempts_send = Arc::clone(&attempts_send);
            async move {
                let now = clock_send.lock().expect("clock").0;
                let count = {
                    let mut list = attempts_send.lock().expect("attempts");
                    list.push(now);
                    list.len()
                };
                if count == 2 {
                    return Err(doudou_core::DoudouError::new(
                        "TEST_WRITE_FAILED",
                        "fixture failure",
                    ));
                }
                Ok(json!({ "status": "sent", "server_message_id": "1" }))
            }
        },
        move |seconds| {
            let clock_sleep = Arc::clone(&clock_sleep);
            async move {
                let mut guard = clock_sleep.lock().expect("clock");
                guard.1.push(seconds);
                guard.0 += seconds;
            }
        },
    )
    .await
    .expect("batch returns summary on item failure");

    // Item 0 already sent before item 1 failed pre-send, so overall batch plan MUST finalize
    // as `unknown` (`PlanStatus::Unknown`) rather than `failed` (`FailedBeforeExternalSend`).
    assert_eq!(result["status"], "unknown");
    let persisted = load_plan(&home, &root, "work", &plan_id, Some("chat.send.batch"))
        .expect("load batch plan");
    assert_eq!(persisted.status, PlanStatus::Unknown);
    assert_eq!(*attempts.lock().expect("attempts"), vec![0.0, 5.0]);
    assert_eq!(clock.lock().expect("clock").1, vec![5.0]);
    let statuses: Vec<&str> = result["results"]
        .as_array()
        .expect("results")
        .iter()
        .filter_map(|r| r["status"].as_str())
        .collect();
    assert_eq!(statuses, vec!["sent", "failed", "skipped"]);
}

#[tokio::test]
async fn batch_first_item_presend_failure_finalizes_failed_and_skips_remaining() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");

    let plan_id = make_batch_plan(&home, &root, 3);
    let result = execute_batch_with(
        &home,
        &root,
        "work",
        &plan_id,
        |_subplan| async {
            Err(doudou_core::DoudouError::new(
                "COMPOSER_PLAN_MISMATCH",
                "failed before first send",
            ))
        },
        |_seconds| async {},
    )
    .await
    .expect("batch returns summary on first-item pre-send failure");

    assert_eq!(result["status"], "failed");
    let persisted = load_plan(&home, &root, "work", &plan_id, Some("chat.send.batch"))
        .expect("load batch plan");
    assert_eq!(persisted.status, PlanStatus::Failed);
    let statuses: Vec<&str> = result["results"]
        .as_array()
        .expect("results")
        .iter()
        .filter_map(|r| r["status"].as_str())
        .collect();
    assert_eq!(statuses, vec!["failed", "skipped", "skipped"]);
}

/// Regression tests for Findings 1 & 2 (`FailedBeforeExternalSend`):
/// - Post-send readback failures in `execute_send`, `execute_like`, and `execute_interaction`
///   carry `write_started: true` and finalize the plan as `PlanStatus::Unknown` (never `Failed`).
/// - Post-send `write_receipt` failures in `execute_like` and `execute_interaction` trigger
///   `PlanExecutionGuard::drop` -> `PlanStatus::Unknown` (never `Failed`) and carry
///   `write_started: true`.
#[tokio::test]
async fn post_send_readback_and_receipt_failures_mark_plans_unknown_not_failed() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    let (_, paths) = create_profile(&home, &root, "work", None).expect("profile");

    // 1. execute_send_with_readback: post-send readback failure -> Unknown + write_started=true.
    let send_plan = create_plan(
        &home,
        &root,
        "work",
        "chat.send",
        json!({
            "conversation": { "conversation_id": "c1", "display_name": "Alpha" },
            "mode": "text",
            "text": "hello",
        }),
        DEFAULT_PLAN_TTL_SECONDS,
    )
    .expect("send plan");
    let send_err = execute_send_with_readback(
        &home,
        &root,
        "work",
        &send_plan.plan_id,
        |_p| async {
            Ok(json!({
                "status": "sent",
                "verification": { "server_message_id": "m-1" },
            }))
        },
        |_p| async {
            Err(doudou_core::DoudouError::new(
                "IO_ERROR",
                "readback io fail",
            ))
        },
    )
    .await
    .expect_err("readback failure must error");
    assert!(
        send_err.is_write_started(),
        "details: {:?}",
        send_err.details
    );
    assert_eq!(
        load_plan(&home, &root, "work", &send_plan.plan_id, Some("chat.send"))
            .expect("load send plan")
            .status,
        PlanStatus::Unknown
    );

    // 2. execute_like_with_readback: post-send readback failure -> Unknown + write_started=true.
    let like_plan = create_plan(
        &home,
        &root,
        "work",
        "chat.message.like",
        json!({
            "conversation": { "conversation_id": "c1", "display_name": "Alpha" },
            "message": { "message_id": "m-1", "text": "hi", "create_time_ms": 1_700_000_000_000_i64 },
            "self_uid": "u-self",
            "already_liked": false,
        }),
        DEFAULT_PLAN_TTL_SECONDS,
    )
    .expect("like plan");
    let like_rb_err = execute_like_with_readback(
        &home,
        &root,
        "work",
        &like_plan.plan_id,
        |_p| async { Ok(json!({ "status": "liked" })) },
        |_p, _since| async {
            Err(doudou_core::DoudouError::new(
                "CDP_ERROR",
                "readback failed after like",
            ))
        },
    )
    .await
    .expect_err("like readback failure must error");
    assert!(
        like_rb_err.is_write_started(),
        "details: {:?}",
        like_rb_err.details
    );
    assert_eq!(
        load_plan(
            &home,
            &root,
            "work",
            &like_plan.plan_id,
            Some("chat.message.like")
        )
        .expect("load like plan")
        .status,
        PlanStatus::Unknown
    );

    // 3. execute_like_with_readback: already_liked readback failure -> Failed (no write started).
    let already_plan = create_plan(
        &home,
        &root,
        "work",
        "chat.message.like",
        json!({
            "conversation": { "conversation_id": "c1", "display_name": "Alpha" },
            "message": { "message_id": "m-1", "text": "hi" },
            "self_uid": "u-self",
            "already_liked": true,
        }),
        DEFAULT_PLAN_TTL_SECONDS,
    )
    .expect("already_liked plan");
    let already_err = execute_like_with_readback(
        &home,
        &root,
        "work",
        &already_plan.plan_id,
        |_p| async { Ok(json!({ "status": "already_liked" })) },
        |_p, _since| async {
            Err(doudou_core::DoudouError::new(
                "CDP_ERROR",
                "readback failed on already_liked",
            ))
        },
    )
    .await
    .expect_err("already_liked readback failure must error");
    assert!(!already_err.is_write_started());
    assert_eq!(
        load_plan(
            &home,
            &root,
            "work",
            &already_plan.plan_id,
            Some("chat.message.like")
        )
        .expect("load already_liked plan")
        .status,
        PlanStatus::Failed
    );

    // 4. execute_interaction_with: post-send recheck failure -> Unknown + write_started=true.
    let video_rb_plan = create_plan(
        &home,
        &root,
        "work",
        "video.comment.create",
        json!({ "sec_uid": "s1", "aweme_id": "v100", "text": "nice" }),
        DEFAULT_PLAN_TTL_SECONDS,
    )
    .expect("video plan");
    let video_rb_err = execute_interaction_with(
        &home,
        &root,
        "work",
        "video.comment.create",
        &video_rb_plan.plan_id,
        |_p| async {
            Ok(json!({
                "status": "created",
                "created_comment": { "cid": "c-9", "text": "nice" },
            }))
        },
        |_p, _res| async {
            Err(doudou_core::DoudouError::new(
                "IO_ERROR",
                "stored recheck failed",
            ))
        },
    )
    .await
    .expect_err("video recheck failure must error");
    assert!(
        video_rb_err.is_write_started(),
        "details: {:?}",
        video_rb_err.details
    );
    assert_eq!(
        load_plan(
            &home,
            &root,
            "work",
            &video_rb_plan.plan_id,
            Some("video.comment.create")
        )
        .expect("load video rb plan")
        .status,
        PlanStatus::Unknown
    );

    // 5. Post-send write_receipt failures in execute_like and execute_interaction MUST drop
    //    PlanExecutionGuard -> PlanStatus::Unknown (NEVER Failed) and set write_started=true.
    std::fs::create_dir_all(&paths.interactions).expect("interactions dir");
    std::fs::write(paths.interactions.join("chat"), b"block-dir").expect("block chat receipts");
    std::fs::write(paths.interactions.join("video"), b"block-dir").expect("block video receipts");

    let like_receipt_plan = create_plan(
        &home,
        &root,
        "work",
        "chat.message.like",
        json!({
            "conversation": { "conversation_id": "c1", "display_name": "Alpha" },
            "message": { "message_id": "m-2", "text": "ok" },
            "self_uid": "u-self",
            "already_liked": false,
        }),
        DEFAULT_PLAN_TTL_SECONDS,
    )
    .expect("like receipt plan");
    let like_receipt_err = execute_like_with_readback(
        &home,
        &root,
        "work",
        &like_receipt_plan.plan_id,
        |_p| async { Ok(json!({ "status": "liked" })) },
        |_p, _since| async {
            Ok((
                json!({ "run_id": "r-1" }),
                vec![json!({
                    "message_id": "m-2",
                    "property_list": { "e:like": [{ "uid": "u-self" }] },
                })],
            ))
        },
    )
    .await
    .expect_err("like write_receipt failure must error");
    assert!(
        like_receipt_err.is_write_started(),
        "details: {:?}",
        like_receipt_err.details
    );
    assert_eq!(
        load_plan(
            &home,
            &root,
            "work",
            &like_receipt_plan.plan_id,
            Some("chat.message.like")
        )
        .expect("load like receipt plan")
        .status,
        PlanStatus::Unknown
    );

    let video_receipt_plan = create_plan(
        &home,
        &root,
        "work",
        "video.comment.create",
        json!({ "sec_uid": "s1", "aweme_id": "v100", "text": "great" }),
        DEFAULT_PLAN_TTL_SECONDS,
    )
    .expect("video receipt plan");
    let video_receipt_err = execute_interaction_with(
        &home,
        &root,
        "work",
        "video.comment.create",
        &video_receipt_plan.plan_id,
        |_p| async {
            Ok(json!({
                "status": "created",
                "created_comment": { "cid": "c-10", "text": "great" },
            }))
        },
        |_p, _res| async { Ok(json!({ "passed": true })) },
    )
    .await
    .expect_err("video write_receipt failure must error");
    assert!(
        video_receipt_err.is_write_started(),
        "details: {:?}",
        video_receipt_err.details
    );
    assert_eq!(
        load_plan(
            &home,
            &root,
            "work",
            &video_receipt_plan.plan_id,
            Some("video.comment.create")
        )
        .expect("load video receipt plan")
        .status,
        PlanStatus::Unknown
    );
}

fn established_conv() -> Value {
    json!({
        "conversation_id": "known",
        "conversation_short_id": "10",
        "conversation_type": 1,
        "kind": "direct",
        "display_name": "Known",
        "participants": { "participants": [], "has_more": false, "cursor": "0" },
    })
}

fn stranger_conv() -> Value {
    json!({
        "conversation_id": "request",
        "conversation_short_id": "20",
        "conversation_type": 1,
        "kind": "direct",
        "inbox": "request",
        "display_name": "Request",
        "participants": { "participants": [], "has_more": false, "cursor": "0" },
        "last_message": {
            "message_id": "last",
            "conversation_id": "request",
            "create_time_ms": 1,
            "message_type": 7,
            "sender": { "sec_uid": "sec-request" },
            "unknown_fields": [],
        },
    })
}

fn encode_stranger_response(command: u32, status: u64, payload: Vec<u8>) -> Vec<u8> {
    let body_field = doudou_protocol::stranger_body_field(command).expect("field");
    encode_fields(&[
        varint_field(3, status),
        string_field(4, "OK"),
        bytes_field(6, encode_fields(&[bytes_field(body_field, payload)])),
    ])
}

fn encode_stranger_conversations_payload(
    short_id: u64,
    conv_id: &str,
    next_cursor: u64,
    has_more: bool,
) -> Vec<u8> {
    let last_msg = encode_fields(&[
        string_field(1, conv_id),
        varint_field(2, 1),
        varint_field(3, 100),
        varint_field(4, 1),
        varint_field(5, 7),
        string_field(8, r#"{"text":"hi stranger"}"#),
        varint_field(10, 1_700_000_000_000),
        string_field(18, "sec-request"),
    ]);
    let conv = encode_fields(&[
        varint_field(1, short_id),
        varint_field(2, 1),
        bytes_field(3, last_msg),
        string_field(4, conv_id),
    ]);
    encode_fields(&[
        varint_field(1, next_cursor),
        varint_field(2, u64::from(has_more)),
        varint_field(3, 1),
        bytes_field(4, conv),
    ])
}

fn encode_init_conversation(conv_id: &str, short_id: u64) -> Vec<u8> {
    let conv_core = encode_fields(&[
        string_field(1, conv_id),
        varint_field(2, short_id),
        varint_field(3, 1),
        string_field(5, "Known"),
    ]);
    let conv_info = encode_fields(&[bytes_field(1, conv_core)]);
    encode_fields(&[bytes_field(1, conv_info)])
}

#[tokio::test]
async fn stranger_refresh_failure_and_stalled_cursor_preserve_previous_index() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");
    let paths = profile_paths(&home, &root, "work").expect("paths");

    let previous = write_conversation_index(&home, &root, "work", &[established_conv()])
        .expect("write initial index");

    // 1. Failure on cmd 1001 preserves previous index.
    let fake = FakeBrowserSession::authenticated(paths.clone());
    let init_resp = encode_init_conversation("known", 10);
    fake.fake_page()
        .push_captured_exchange(CapturedNetworkExchange {
            request_id: "init-2".to_owned(),
            url: "https://imapi.douyin.com/v1/message/get_message_by_init".to_owned(),
            method: "POST".to_owned(),
            status: Some(200),
            mime_type: Some("application/x-protobuf".to_owned()),
            request_headers: BTreeMap::new(),
            response_headers: BTreeMap::new(),
            request_body: Some(encode_fields(&[varint_field(1, 200)])),
            response_body: Some(init_resp.clone()),
            resource_type: Some("Fetch".to_owned()),
            timestamp: doudou_core::utc_now(),
        });
    fake.fake_page().push_fetch_result(
        STRANGER_CONVERSATIONS_ENDPOINT,
        Ok(CdpFetchResponse {
            status: 500,
            ok: false,
            headers: BTreeMap::new(),
            body_text: None,
            body_base64: Some(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                encode_stranger_response(1001, 1, vec![]),
            )),
        }),
    );

    let mut session = fake.into_chrome_session();
    let err = refresh_session_index(&mut session, &home, &root, "work", true)
        .await
        .expect_err("stranger failure must error");
    assert_eq!(err.code, "STRANGER_CONVERSATIONS_FAILED");
    assert_eq!(
        read_conversation_index(&home, &root, "work")
            .expect("read")
            .expect("present"),
        previous
    );

    // 2. Stalled cursor on cmd 1001 fails and preserves previous index.
    let fake2 = FakeBrowserSession::authenticated(paths);
    fake2
        .fake_page()
        .push_captured_exchange(CapturedNetworkExchange {
            request_id: "init-3".to_owned(),
            url: "https://imapi.douyin.com/v1/message/get_message_by_init".to_owned(),
            method: "POST".to_owned(),
            status: Some(200),
            mime_type: Some("application/x-protobuf".to_owned()),
            request_headers: BTreeMap::new(),
            response_headers: BTreeMap::new(),
            request_body: Some(encode_fields(&[varint_field(1, 200)])),
            response_body: Some(init_resp),
            resource_type: Some("Fetch".to_owned()),
            timestamp: doudou_core::utc_now(),
        });
    let stalled_payload = encode_stranger_conversations_payload(20, "request", 0, true);
    fake2.fake_page().push_fetch_result(
        STRANGER_CONVERSATIONS_ENDPOINT,
        Ok(CdpFetchResponse {
            status: 200,
            ok: true,
            headers: BTreeMap::new(),
            body_text: None,
            body_base64: Some(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                encode_stranger_response(1001, 0, stalled_payload),
            )),
        }),
    );
    let mut session2 = fake2.into_chrome_session();
    let stalled_err = refresh_session_index(&mut session2, &home, &root, "work", true)
        .await
        .expect_err("stalled cursor must error");
    assert_eq!(stalled_err.code, "STRANGER_PAGE_CURSOR_STALLED");
    assert_eq!(
        read_conversation_index(&home, &root, "work")
            .expect("read")
            .expect("present"),
        previous
    );
}

#[tokio::test]
async fn stranger_collect_routes_1002_and_falls_back_to_summary_when_empty() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");
    let paths = profile_paths(&home, &root, "work").expect("paths");

    // Seed fresh session index snapshot + conversation index with a stranger conversation.
    let mut req_conv = stranger_conv();
    req_conv["last_message"]["message_id"] = json!("2");
    req_conv["last_message"]["text"] = json!("Visible request message");
    write_conversation_index(&home, &root, "work", &[req_conv]).expect("write index");
    doudou_storage::write_session_index_snapshot(
        &home,
        &root,
        "work",
        &json!({
            "schema": "doudou.session-index-snapshot.v1",
            "completed_at": doudou_core::utc_now(),
            "conversation_count": 1,
            "inbox_count": 1,
        }),
    )
    .expect("snapshot");

    let fake = FakeBrowserSession::authenticated(paths);
    fake.fake_page().push_fetch_result(
        STRANGER_MESSAGES_ENDPOINT,
        Ok(CdpFetchResponse {
            status: 200,
            ok: true,
            headers: BTreeMap::new(),
            body_text: None,
            body_base64: Some(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                encode_stranger_response(1002, 0, Vec::new()),
            )),
        }),
    );

    let mut session = fake.into_chrome_session();
    let run = collect_messages(
        &mut session,
        &home,
        &root,
        "work",
        "request",
        None,
        None,
        10,
        false,
    )
    .await
    .expect("collect stranger summary");

    assert_eq!(run["message_count"], 1);
    assert_eq!(run["pages"][0]["protocol_received"], 0);
    assert_eq!(run["pages"][0]["summary_received"], 1);
    assert_eq!(run["pages"][0]["collection_source"], "conversation_summary");
    assert_eq!(run["pages"][0]["fallback_used"], false);
}

#[test]
fn stranger_display_name_filtering_wire_format_and_readonly_write_guard() {
    // 1. Inbox filtering does not mutate index.
    let filtered = filter_conversations(&[established_conv(), stranger_conv()], "all", "request");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0]["conversation_id"], "request");

    // 2. Counterparty nickname is used when self_uid is strictly inferred, NEVER self's name.
    let mut known = established_conv();
    known["participants"] = json!({
        "participants": [
            { "uid": "self", "sec_uid": "sec-self", "display_name": "My Account" },
            { "uid": "known-other", "sec_uid": "sec-known", "display_name": "Known" },
        ],
        "has_more": false,
        "cursor": "0",
    });
    let mut req = stranger_conv();
    req["participants"] = json!({
        "participants": [
            { "uid": "self", "sec_uid": "sec-self", "display_name": Value::Null },
            { "uid": "other", "sec_uid": "sec-request", "display_name": Value::Null },
        ],
        "has_more": false,
        "cursor": "0",
    });
    let mut ids = HashMap::new();
    ids.insert(
        "sec-self".to_owned(),
        json!({ "uid": "self", "sec_uid": "sec-self", "display_name": "My Account" }),
    );
    ids.insert(
        "sec-request".to_owned(),
        json!({ "uid": "other", "sec_uid": "sec-request", "display_name": "Counterparty Nickname" }),
    );

    let mut both = vec![known, req.clone()];
    apply_conversation_identities(&mut both, &ids);
    assert_eq!(both[1]["display_name"], "Counterparty Nickname");

    // When self_uid cannot be inferred (single direct conversation), display_name stays original.
    let mut only_req = vec![req];
    apply_conversation_identities(&mut only_req, &ids);
    assert_eq!(only_req[0]["display_name"], "Request");

    // 3. Command 1002 uses body field 1001 and reset_unread_count = 0.
    let body = stranger_messages_request(20, false);
    let env = stranger_request(&[], 1002, &body, Some(1)).expect("stranger envelope");
    let outer = decode_fields(&env).expect("decode outer");
    let inner = decode_fields(
        get_field(&outer, 8, Some(2))
            .expect("f8")
            .as_bytes()
            .expect("bytes"),
    )
    .expect("decode inner");
    let cmd_body = get_field(&inner, 1001, Some(2))
        .expect("f1001")
        .as_bytes()
        .expect("bytes");
    assert_eq!(cmd_body, body.as_slice());
    let decoded_body = decode_fields(&body).expect("decode body");
    assert_eq!(
        get_field(&decoded_body, 2, Some(0))
            .expect("f2")
            .as_varint(),
        Some(0)
    );

    // 4. Write plans reject stranger inbox before creating any plan file.
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");
    let req_c = stranger_conv();

    assert_eq!(
        plan_send_for_conversation(&home, &root, "work", &req_c, &json!({ "text": "x" }))
            .expect_err("send rejected")
            .code,
        "STRANGER_WRITE_UNSUPPORTED"
    );
    assert_eq!(
        plan_batch_from_conversations(
            &home,
            &root,
            "work",
            std::slice::from_ref(&req_c),
            &json!({ "targets": ["request"], "text": "x" })
        )
        .expect_err("batch rejected")
        .code,
        "STRANGER_WRITE_UNSUPPORTED"
    );
    assert_eq!(
        plan_like_for_conversation(
            &home,
            &root,
            "work",
            std::slice::from_ref(&req_c),
            &req_c,
            &json!({ "message": "1" })
        )
        .expect_err("like rejected")
        .code,
        "STRANGER_WRITE_UNSUPPORTED"
    );
}

#[test]
fn video_interaction_plans_and_readback_verification_work() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");

    write_video(
        &home,
        &root,
        "work",
        &json!({ "aweme_id": "v100", "desc": "demo" }),
        None,
        &[json!({
            "cid": "c1",
            "reply_id": "c1",
            "text": "original comment",
            "user_digged": true,
            "digg_count": 3,
            "user": { "nickname": "Alice" },
        })],
        &[],
    )
    .expect("write video");

    let create_p = plan_comment(
        &home,
        &root,
        "work",
        &json!({ "sec_uid": "s1", "aweme_id": "v100", "text": "nice!" }),
    )
    .expect("plan comment");
    assert_eq!(create_p["status"], "confirmation_required");

    let reply_p = plan_reply(
        &home,
        &root,
        "work",
        &json!({
            "sec_uid": "s1",
            "aweme_id": "v100",
            "text": "thanks",
            "comment": { "cid": "c1", "text": "original comment" },
        }),
    )
    .expect("plan reply");
    assert_eq!(reply_p["plan"]["operation"], "video.comment.reply");

    let like_p = plan_comment_like(
        &home,
        &root,
        "work",
        &json!({
            "sec_uid": "s1",
            "aweme_id": "v100",
            "comment": { "cid": "c1" },
        }),
    )
    .expect("plan like");
    assert_eq!(like_p["plan"]["operation"], "video.comment.like");

    let like_ver = verify_interaction_readback(
        "video.comment.like",
        &like_p["plan"]["payload"],
        &json!({}),
        &[json!({ "cid": "c1", "user_digged": true })],
    );
    assert_eq!(like_ver["passed"], true);

    let send_ver = verify_send_readback(
        &json!({
            "conversation": { "conversation_id": "conv-1" },
            "mode": "text",
            "text": "hello",
        }),
        &json!({ "verification": { "server_message_id": "m1" } }),
        &[json!({
            "message_id": "m1",
            "conversation_id": "conv-1",
            "message_type": 7,
            "text": "hello",
        })],
    );
    assert_eq!(send_ver["passed"], true);
}

#[tokio::test]
async fn cdp_ui_page_bridge_translates_queries_clicks_keys_and_network_capture() {
    let fake = Arc::new(FakeCdpPage::new());
    fake.set_url("https://www.douyin.com/chat");
    fake.push_eval_result("querySelectorAll", json!(["el-1"]));
    fake.push_eval_result(
        "getBoundingClientRect",
        json!({ "x": 10.0, "y": 20.0, "width": 100.0, "height": 40.0 }),
    );

    let ui = CdpUiPage::new(Arc::clone(&fake) as Arc<dyn doudou_cdp::CdpPageOps>);
    assert_eq!(ui.url().await.expect("url"), "https://www.douyin.com/chat");

    let nodes = ui
        .query_all(None, &SelectorQuery::css("div.item"), 10)
        .await
        .expect("query");
    assert_eq!(nodes.len(), 1);

    let bbox = ui
        .bounding_box(&nodes[0])
        .await
        .expect("bbox")
        .expect("some");
    assert!((bbox.x - 10.0).abs() < f64::EPSILON);

    ui.click_element(&nodes[0], MouseButton::Left, 1_000)
        .await
        .expect("click");
    assert_eq!(fake.clicks().len(), 1);
    assert!((fake.clicks()[0].x - 60.0).abs() < f64::EPSILON);

    ui.press_key("Control+A", 1_000).await.expect("press key");
    let keys = fake.dispatched_keys();
    assert_eq!(keys.len(), 1);
    assert!(keys[0].modifiers.ctrl);
    assert_eq!(keys[0].code.as_deref(), Some("KeyA"));

    let spec = parse_key_spec("Shift+Enter");
    assert!(spec.modifiers.shift);
    assert_eq!(spec.key, "Enter");

    let handle = ui
        .start_network_capture(CaptureFilter::contains(["/v1/message/send"]))
        .await
        .expect("capture");
    fake.push_captured_exchange(CapturedNetworkExchange {
        request_id: "r1".to_owned(),
        url: "https://imapi.douyin.com/v1/message/send".to_owned(),
        method: "POST".to_owned(),
        status: Some(200),
        mime_type: Some("application/x-protobuf".to_owned()),
        request_headers: BTreeMap::new(),
        response_headers: BTreeMap::new(),
        request_body: Some(b"req".to_vec()),
        response_body: Some(b"resp".to_vec()),
        resource_type: Some("Fetch".to_owned()),
        timestamp: doudou_core::utc_now(),
    });
    let snap = handle.stop().await.expect("stop");
    assert_eq!(snap.requests.len(), 1);
    assert_eq!(snap.responses.len(), 1);
    assert_eq!(snap.responses[0].body, b"resp");
}
