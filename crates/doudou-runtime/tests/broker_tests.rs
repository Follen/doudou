//! Broker integration tests: lifecycle, auth, idle timeout, 16 MiB frame limits,
//! global registry, read verification recovery, and formal at-most-once write safety.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use doudou_cdp::{FakeBrowserSession, FakeCdpPage, SessionMode};
use doudou_core::{DoudouError, profile_paths};
use doudou_runtime::features::chat::{execute_send_with, execute_send_with_readback};
use doudou_runtime::{
    BROKER_REGISTRATION_SCHEMA, BROKER_RESPONSE_LIMIT_BYTES, BrokerClient, BrokerDescriptor,
    BrokerEndpoint, BrokerServer, BrokerServerConfig, CommandSemantics, FnBrokerHandler,
    MAX_FRAME_BYTES, broker_status, command_timeout_seconds, list_brokers, register_broker,
    stop_all_brokers, stop_broker, unregister_broker,
};
use doudou_storage::{
    DEFAULT_PLAN_TTL_SECONDS, PlanStatus, create_plan, create_profile, load_plan,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

#[test]
fn long_collection_commands_have_bounded_total_timeouts() {
    assert!((command_timeout_seconds("videos.collect", &json!({})) - 600.0).abs() < f64::EPSILON);
    assert!((command_timeout_seconds("video.collect", &json!({})) - 900.0).abs() < f64::EPSILON);
    assert!((command_timeout_seconds("messages.collect", &json!({})) - 90.0).abs() < f64::EPSILON);
    assert!((command_timeout_seconds("page.doctor", &json!({})) - 180.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn broker_reuses_session_and_idle_exit_cleans_runtime() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    let (_, paths) = create_profile(&home, &root, "work", None).expect("create profile");

    let config = BrokerServerConfig::new(home.clone(), root.clone(), "work")
        .with_fake_browser(true)
        .with_idle_timeout(Duration::from_millis(200));
    let server = BrokerServer::start(config, None)
        .await
        .expect("start broker");

    let client = BrokerClient::new(&home, &root, "work").expect("client");
    let first = client
        .request_async("status", json!({}), CommandSemantics::Read, None, false)
        .await
        .expect("status 1")["result"]
        .clone();
    let second = client
        .request_async("ping", json!({}), CommandSemantics::Read, None, false)
        .await
        .expect("ping")["result"]
        .clone();
    let third = client
        .request_async("status", json!({}), CommandSemantics::Read, None, false)
        .await
        .expect("status 2")["result"]
        .clone();
    let ready = client
        .request_async(
            "session.refresh",
            json!({}),
            CommandSemantics::Control,
            None,
            false,
        )
        .await
        .expect("session.refresh")["result"]
        .clone();
    let reused = client
        .request_async(
            "session.status",
            json!({}),
            CommandSemantics::Read,
            None,
            false,
        )
        .await
        .expect("session.status")["result"]
        .clone();

    assert_eq!(first["pid"], second["pid"]);
    assert_eq!(second["pid"], third["pid"]);
    assert_eq!(
        first["browser"]["started_at"],
        third["browser"]["started_at"]
    );
    assert_eq!(ready["initialization_count"], 1);
    assert_eq!(reused["browser"]["context_generation"], 1);
    assert!(paths.lock.exists());
    assert!(paths.broker_descriptor.exists());
    assert!(paths.broker_token.exists());

    // Wait for automatic idle exit and confirm all runtime files and locks are cleaned up.
    tokio::time::timeout(Duration::from_secs(3), server.wait_stopped())
        .await
        .expect("broker should exit on idle timeout");
    assert!(!paths.broker_descriptor.exists());
    assert!(!paths.broker_token.exists());
    assert!(!paths.lock.exists());
}

#[tokio::test]
async fn invalid_token_and_oversized_request_frame_are_rejected_cleanly() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("create profile");

    let config = BrokerServerConfig::new(home.clone(), root.clone(), "work")
        .with_fake_browser(true)
        .with_idle_timeout(Duration::from_secs(30));
    let server = BrokerServer::start(config, None)
        .await
        .expect("start broker");
    let endpoint = server.endpoint().clone();

    // 1. Invalid auth token returns BROKER_AUTH_FAILED.
    {
        let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port))
            .await
            .expect("connect");
        stream
            .write_all(
                br#"{"token":"wrong","request_id":"bad","command":"ping","semantics":"read"}"#,
            )
            .await
            .expect("write");
        stream.write_all(b"\n").await.expect("newline");
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read");
        let resp: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(resp["ok"], false);
        assert_eq!(resp["error"]["code"], "BROKER_AUTH_FAILED");
    }

    // 2. Oversized request line (> 16 MiB) returns BROKER_FRAME_TOO_LARGE cleanly.
    {
        let stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port))
            .await
            .expect("connect");
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);

        // Read concurrently with writing so Windows TCP does not RST unread response bytes
        // when the server closes the connection immediately after rejecting >16 MiB.
        let writer_task = tokio::spawn(async move {
            let chunk = vec![b'a'; 64 * 1024];
            let chunks_needed = (MAX_FRAME_BYTES / chunk.len()) + 2;
            for _ in 0..chunks_needed {
                if write_half.write_all(&chunk).await.is_err() {
                    break;
                }
            }
            let _ = write_half.write_all(b"\n").await;
            let _ = write_half.flush().await;
        });

        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read error frame");
        let _ = writer_task.await;
        let resp: Value = serde_json::from_str(&line).expect("json error response");
        assert_eq!(resp["ok"], false);
        assert_eq!(resp["error"]["code"], "BROKER_FRAME_TOO_LARGE");
        assert_eq!(resp["error"]["details"]["limit_bytes"], MAX_FRAME_BYTES);
    }

    server.close().await;
}

async fn run_raw_frame_server(
    response: Vec<u8>,
) -> (u16, Arc<Notify>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind mock tcp");
    let port = listener.local_addr().expect("local addr").port();
    let closed = Arc::new(Notify::new());
    let closed_clone = Arc::clone(&closed);

    let handle = tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let (read_half, mut write_half) = stream.into_split();
            let mut reader = BufReader::new(read_half);
            let mut req_line = String::new();
            let _ = reader.read_line(&mut req_line).await;
            let _ = write_half.write_all(&response).await;
            let _ = write_half.flush().await;
            let mut drain = String::new();
            let _ = reader.read_line(&mut drain).await;
            let _ = write_half.shutdown().await;
        }
        closed_clone.notify_waiters();
    });
    (port, closed, handle)
}

#[tokio::test]
async fn client_frame_limits_accept_128kib_and_reject_oversized_read_and_write() {
    // 1. Accepts response larger than 128 KiB.
    let blob = "x".repeat(128 * 1024);
    let mut large_ok =
        serde_json::to_vec(&json!({ "ok": true, "result": { "blob": blob } })).expect("serialize");
    large_ok.push(b'\n');
    let (port, closed, handle) = run_raw_frame_server(large_ok).await;
    let res = BrokerClient::send_to_endpoint(
        "127.0.0.1",
        port,
        "test-token",
        "large.read",
        json!({}),
        CommandSemantics::Read,
        None,
    )
    .await
    .expect("128 KiB response must succeed");
    assert_eq!(
        res["result"]["blob"].as_str().map(str::len),
        Some(128 * 1024)
    );
    tokio::time::timeout(Duration::from_secs(2), closed.notified())
        .await
        .expect("connection closed");
    handle.abort();

    // 2. Read response over 16 MiB returns BROKER_RESPONSE_TOO_LARGE.
    let mut oversized = vec![b'x'; BROKER_RESPONSE_LIMIT_BYTES + 1];
    oversized.push(b'\n');
    let (port2, closed2, handle2) = run_raw_frame_server(oversized.clone()).await;
    let read_err = BrokerClient::send_to_endpoint(
        "127.0.0.1",
        port2,
        "test-token",
        "oversized.read",
        json!({}),
        CommandSemantics::Read,
        None,
    )
    .await
    .expect_err("oversized read must fail");
    assert_eq!(read_err.code, "BROKER_RESPONSE_TOO_LARGE");
    assert_eq!(
        read_err.details.expect("details")["response_limit_bytes"],
        BROKER_RESPONSE_LIMIT_BYTES
    );
    tokio::time::timeout(Duration::from_secs(2), closed2.notified())
        .await
        .expect("connection closed");
    handle2.abort();

    // 3. Write response over 16 MiB returns WRITE_RESULT_UNKNOWN with write_started=true.
    let (port3, closed3, handle3) = run_raw_frame_server(oversized).await;
    let write_err = BrokerClient::send_to_endpoint(
        "127.0.0.1",
        port3,
        "test-token",
        "oversized.write",
        json!({}),
        CommandSemantics::Write,
        Some("write-large-frame"),
    )
    .await
    .expect_err("oversized write must fail");
    assert_eq!(write_err.code, "WRITE_RESULT_UNKNOWN");
    let details = write_err.details.expect("details");
    assert_eq!(details["request_id"], "write-large-frame");
    assert_eq!(details["write_started"], true);
    tokio::time::timeout(Duration::from_secs(2), closed3.notified())
        .await
        .expect("connection closed");
    handle3.abort();
}

#[tokio::test]
async fn broker_registration_lists_queries_and_stops_brokers() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    let (_, paths) = create_profile(&home, &root, "work", None).expect("profile");

    // Manual registration round-trip.
    let desc = BrokerDescriptor {
        schema: doudou_runtime::BROKER_DESCRIPTOR_SCHEMA.to_owned(),
        pid: std::process::id(),
        process_start: doudou_runtime::current_process_identity().start,
        data_root: root.display().to_string(),
        profile_id: paths.profile_id.clone(),
        endpoint: BrokerEndpoint {
            transport: "tcp".to_owned(),
            host: "127.0.0.1".to_owned(),
            port: 12345,
        },
        token_file: "broker.token".to_owned(),
        mode: "fake".to_owned(),
        browser_version: None,
        created_at: doudou_core::utc_now(),
        last_activity_at: doudou_core::utc_now(),
        takeover: None,
    };
    let reg_file = register_broker(&paths, &desc).expect("register");
    let reg_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&reg_file).expect("read reg"))
            .expect("parse reg");
    assert_eq!(reg_json["schema"], BROKER_REGISTRATION_SCHEMA);
    assert_eq!(list_brokers(&home).expect("list").len(), 1);
    unregister_broker(Some(&reg_file));
    assert!(!reg_file.exists());

    // Live broker status + stop_broker + stop_all_brokers.
    let config = BrokerServerConfig::new(home.clone(), root.clone(), "work")
        .with_fake_browser(true)
        .with_idle_timeout(Duration::from_secs(60));
    let server = BrokerServer::start(config, None).await.expect("start");

    let live_list = list_brokers(&home).expect("list live");
    assert_eq!(live_list.len(), 1);
    assert_eq!(live_list[0]["profile_id"], "work");

    let st = broker_status(&home, &root, "work").await.expect("status");
    assert_eq!(st["running"], true);

    let stopped = stop_broker(&home, &root, "work").await.expect("stop");
    assert_eq!(stopped["stopped"], true);
    assert_eq!(stopped["was_running"], true);
    server.wait_stopped().await;

    assert_eq!(list_brokers(&home).expect("empty after stop").len(), 0);

    // Start again and verify stop_all_brokers stops it cleanly.
    let config2 = BrokerServerConfig::new(home.clone(), root.clone(), "work")
        .with_fake_browser(true)
        .with_idle_timeout(Duration::from_secs(60));
    let server2 = BrokerServer::start(config2, None).await.expect("start 2");
    let all_stopped = stop_all_brokers(&home).await.expect("stop all");
    assert_eq!(all_stopped["stopped_count"], 1);
    server2.wait_stopped().await;
}

#[tokio::test]
async fn deterministic_write_errors_and_timeouts_are_stable_and_release_lock() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");

    let handler = Arc::new(FnBrokerHandler::new(
        |command, _params, _session, _home, _root, _profile| async move {
            match command.as_str() {
                "drift.write" => Err(DoudouError::new("PAGE_ADAPTER_DRIFTED", "adapter drifted")
                    .with_details(json!({ "adapter_version": "v1" }))),
                "preclick.write" => Err(DoudouError::new(
                    "COMMENT_COMPOSER_EMPTY",
                    "empty before click",
                )),
                "slow.write" => {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    Ok(Some(json!({ "status": "ok" })))
                }
                _ => Ok(None),
            }
        },
    ));

    let config = BrokerServerConfig::new(home.clone(), root.clone(), "work")
        .with_fake_browser(true)
        .with_idle_timeout(Duration::from_secs(60));
    let server = BrokerServer::start(config, Some(handler))
        .await
        .expect("start");

    // PAGE_ADAPTER_DRIFTED is not hidden as WRITE_RESULT_UNKNOWN.
    let drift = server
        .execute(&json!({
            "request_id": "drift-1",
            "command": "drift.write",
            "semantics": "write",
            "params": {},
        }))
        .await;
    assert_eq!(drift["ok"], false);
    assert_eq!(drift["error"]["code"], "PAGE_ADAPTER_DRIFTED");

    // Pre-click write error without write_started is preserved verbatim.
    let pre = server
        .execute(&json!({
            "request_id": "pre-1",
            "command": "preclick.write",
            "semantics": "write",
            "params": {},
        }))
        .await;
    assert_eq!(pre["ok"], false);
    assert_eq!(pre["error"]["code"], "COMMENT_COMPOSER_EMPTY");

    // Timed-out write returns WRITE_RESULT_UNKNOWN, releases command lock, and is cached idempotently.
    let timeout_req = json!({
        "request_id": "timed-out-write",
        "command": "slow.write",
        "semantics": "write",
        "command_timeout_seconds": 0.05,
        "params": {},
    });
    let first = server.execute(&timeout_req).await;
    let second = server.execute(&timeout_req).await;
    assert_eq!(first, second);
    assert_eq!(first["ok"], false);
    assert_eq!(first["error"]["code"], "WRITE_RESULT_UNKNOWN");
    assert!(!server.is_command_locked());

    server.close().await;
}

#[tokio::test]
async fn kick_without_visible_challenge_does_not_enter_takeover() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");

    let calls = Arc::new(AtomicUsize::new(0));
    let calls_ref = Arc::clone(&calls);
    let handler = Arc::new(FnBrokerHandler::new(
        move |_command, _params, _session, _home, _root, _profile| {
            let calls_ref = Arc::clone(&calls_ref);
            async move {
                calls_ref.fetch_add(1, Ordering::SeqCst);
                Err(
                    DoudouError::new("VERIFICATION_REQUIRED", "verification gateway")
                        .with_details(json!({ "gateway_decision": "KICK" })),
                )
            }
        },
    ));

    let config = BrokerServerConfig::new(home.clone(), root.clone(), "work")
        .with_fake_browser(true)
        .with_idle_timeout(Duration::from_secs(60));
    let server = BrokerServer::start(config, Some(handler))
        .await
        .expect("start");

    let resp = server
        .execute(&json!({
            "request_id": "kick-1",
            "command": "comments.collect",
            "semantics": "read",
            "params": {},
        }))
        .await;
    assert_eq!(resp["ok"], false);
    assert_eq!(resp["error"]["code"], "VERIFICATION_REQUIRED");
    assert_eq!(resp["error"]["details"]["gateway_decision"], "KICK");
    assert_eq!(resp["error"]["details"]["takeover"]["active"], false);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    server.close().await;
}

#[tokio::test]
async fn read_login_recovery_switches_takeover_and_retries_command_once() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");
    let paths = profile_paths(&home, &root, "work").expect("paths");

    let page = Arc::new(FakeCdpPage::new());
    page.set_url("https://www.douyin.com/chat");
    // Verification clears on takeover poll and chat loads.
    page.push_eval_result("second-verify-mask", json!("sms_code"));
    page.push_eval_result("second-verify-mask", json!("sms_code"));
    page.push_eval_result("second-verify-mask", Value::Null);
    page.push_eval_result("conversation-item", json!(false));
    page.push_eval_result("conversation-item", json!(true));
    let session = FakeBrowserSession::from_page(paths, page).into_chrome_session();
    assert_eq!(session.mode(), SessionMode::Fake);

    let calls = Arc::new(AtomicUsize::new(0));
    let calls_ref = Arc::clone(&calls);
    let handler = Arc::new(FnBrokerHandler::new(
        move |_command, _params, _session, _home, _root, _profile| {
            let calls_ref = Arc::clone(&calls_ref);
            async move {
                let n = calls_ref.fetch_add(1, Ordering::SeqCst) + 1;
                if n == 1 {
                    return Err(DoudouError::new("LOGIN_REQUIRED", "login needed"));
                }
                Ok(Some(json!({ "status": "resumed" })))
            }
        },
    ));

    let config = BrokerServerConfig::new(home.clone(), root.clone(), "work")
        .with_fake_browser(true)
        .with_takeover_timeout(Duration::from_secs(3))
        .with_idle_timeout(Duration::from_secs(60));
    let server = BrokerServer::start_with_session(config, session, Some(handler))
        .await
        .expect("start");

    let resp = server
        .execute(&json!({
            "request_id": "recover-1",
            "command": "messages.collect",
            "semantics": "read",
            "params": {},
        }))
        .await;
    assert_eq!(resp["ok"], true, "response: {resp}");
    assert_eq!(resp["result"]["status"], "resumed");
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    server.close().await;
}

/// Formal verification invariant test (`NoConcurrentWrites` + `AtMostOnceSend`):
/// 1. Two concurrent `execute_send` calls with the SAME `plan_id` (different `request_id`s)
///    are serialized by the broker's single-flight mutex and only execute ONCE; the second fails
///    deterministically with `PLAN_ALREADY_USED`.
/// 2. If execution aborts/cancels mid-flight before `complete_plan`, retrying the same `plan_id`
///    is rejected with `PLAN_ALREADY_USED`, proving `AtMostOnceSend`.
#[tokio::test]
async fn broker_enforces_at_most_once_send_under_concurrency_and_midflight_cancellation() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");

    // --- Part 1: Two concurrent execute_send requests for the same plan_id ---
    let plan = create_plan(
        &home,
        &root,
        "work",
        "chat.send",
        json!({
            "conversation": { "conversation_id": "c1", "display_name": "Alpha" },
            "mode": "text",
            "text": "hello once",
        }),
        DEFAULT_PLAN_TTL_SECONDS,
    )
    .expect("create plan");

    let external_send_count = Arc::new(AtomicUsize::new(0));
    let send_counter = Arc::clone(&external_send_count);
    let handler = Arc::new(FnBrokerHandler::new(
        move |command, params, _session, home, root, profile| {
            let send_counter = Arc::clone(&send_counter);
            async move {
                if command != "chat.send.execute" {
                    return Ok(None);
                }
                let plan_id = params["plan_id"].as_str().expect("plan_id");
                let res = execute_send_with(&home, &root, &profile, plan_id, move |_p| {
                    let send_counter = Arc::clone(&send_counter);
                    async move {
                        // Hold briefly to ensure the second concurrent caller queues behind the lock.
                        tokio::time::sleep(Duration::from_millis(80)).await;
                        send_counter.fetch_add(1, Ordering::SeqCst);
                        Ok(json!({
                            "status": "sent",
                            "verification": { "passed": true, "server_message_id": "srv-1" }
                        }))
                    }
                })
                .await?;
                Ok(Some(res))
            }
        },
    ));

    let config = BrokerServerConfig::new(home.clone(), root.clone(), "work")
        .with_fake_browser(true)
        .with_idle_timeout(Duration::from_secs(60));
    let server = Arc::new(
        BrokerServer::start(config, Some(handler))
            .await
            .expect("start broker"),
    );

    let req1 = json!({
        "request_id": "concurrent-req-1",
        "command": "chat.send.execute",
        "semantics": "write",
        "params": { "plan_id": plan.plan_id },
    });
    let req2 = json!({
        "request_id": "concurrent-req-2",
        "command": "chat.send.execute",
        "semantics": "write",
        "params": { "plan_id": plan.plan_id },
    });

    let (r1, r2) = tokio::join!(server.execute(&req1), server.execute(&req2));
    assert_eq!(
        external_send_count.load(Ordering::SeqCst),
        1,
        "external send must execute strictly once"
    );

    let ok_count = usize::from(r1["ok"] == true) + usize::from(r2["ok"] == true);
    assert_eq!(ok_count, 1, "exactly one caller succeeds: r1={r1}, r2={r2}");
    let failed_resp = if r1["ok"] == true { &r2 } else { &r1 };
    assert_eq!(failed_resp["error"]["code"], "PLAN_ALREADY_USED");

    server.close().await;

    // --- Part 2: Mid-flight cancellation / abort before complete_plan ---
    let plan2 = create_plan(
        &home,
        &root,
        "work",
        "chat.send",
        json!({
            "conversation": { "conversation_id": "c1", "display_name": "Alpha" },
            "mode": "text",
            "text": "cancel midflight",
        }),
        DEFAULT_PLAN_TTL_SECONDS,
    )
    .expect("create plan2");

    let started = Arc::new(Notify::new());
    let started_ref = Arc::clone(&started);
    let midflight_sends = Arc::new(AtomicUsize::new(0));
    let midflight_ref = Arc::clone(&midflight_sends);

    let home2 = home.clone();
    let root2 = root.clone();
    let pid2 = plan2.plan_id.clone();
    let handle = tokio::spawn(async move {
        execute_send_with(&home2, &root2, "work", &pid2, move |_p| {
            let started_ref = Arc::clone(&started_ref);
            let midflight_ref = Arc::clone(&midflight_ref);
            async move {
                midflight_ref.fetch_add(1, Ordering::SeqCst);
                started_ref.notify_one();
                // Hang forever simulating a crash/cancellation right after external send starts.
                std::future::pending::<doudou_core::Result<Value>>().await
            }
        })
        .await
    });

    started.notified().await;
    // Abort task mid-flight before complete_plan runs!
    handle.abort();
    let _ = handle.await;

    // Plan must be in Unknown (via Drop guard) or Executing — never ConfirmationRequired.
    let after_abort = load_plan(&home, &root, "work", &plan2.plan_id, Some("chat.send"))
        .expect("load aborted plan");
    assert!(
        matches!(
            after_abort.status,
            PlanStatus::Unknown | PlanStatus::Executing
        ),
        "aborted plan status was {:?}",
        after_abort.status
    );

    // Retrying the same plan_id MUST be rejected with PLAN_ALREADY_USED and never invoke runner.
    let retry_counter = Arc::clone(&midflight_sends);
    let retry_err = execute_send_with(&home, &root, "work", &plan2.plan_id, move |_p| {
        let midflight_ref = Arc::clone(&retry_counter);
        async move {
            midflight_ref.fetch_add(1, Ordering::SeqCst);
            Ok(json!({ "status": "sent" }))
        }
    })
    .await
    .expect_err("retrying aborted plan must fail");
    assert_eq!(retry_err.code, "PLAN_ALREADY_USED");
    assert_eq!(midflight_sends.load(Ordering::SeqCst), 1);
}

/// Regression test for Finding 1: post-send protocol read-back failure in `execute_send`
/// carries `write_started: true`, finalizes the plan as `PlanStatus::Unknown` (never `Failed`),
/// and causes the broker to return `WRITE_RESULT_UNKNOWN`.
#[tokio::test]
async fn post_send_readback_failure_marks_plan_unknown_and_broker_returns_write_result_unknown() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");

    let plan = create_plan(
        &home,
        &root,
        "work",
        "chat.send",
        json!({
            "conversation": { "conversation_id": "c1", "display_name": "Alpha" },
            "mode": "text",
            "text": "hello readback failure",
        }),
        DEFAULT_PLAN_TTL_SECONDS,
    )
    .expect("create plan");

    let handler = Arc::new(FnBrokerHandler::new(
        |command, params, _session, home, root, profile| async move {
            if command != "chat.send.execute" {
                return Ok(None);
            }
            let plan_id = params["plan_id"].as_str().expect("plan_id");
            let res = execute_send_with_readback(
                &home,
                &root,
                &profile,
                plan_id,
                |_p| async {
                    Ok(json!({
                        "status": "sent",
                        "verification": { "passed": true, "server_message_id": "srv-99" },
                    }))
                },
                |_p| async {
                    Err(DoudouError::new(
                        "CDP_ERROR",
                        "readback transport disconnected",
                    ))
                },
            )
            .await?;
            Ok(Some(res))
        },
    ));

    let config = BrokerServerConfig::new(home.clone(), root.clone(), "work")
        .with_fake_browser(true)
        .with_idle_timeout(Duration::from_secs(60));
    let server = BrokerServer::start(config, Some(handler))
        .await
        .expect("start broker");

    let resp = server
        .execute(&json!({
            "request_id": "send-readback-fail-1",
            "command": "chat.send.execute",
            "semantics": "write",
            "params": { "plan_id": plan.plan_id },
        }))
        .await;

    assert_eq!(resp["ok"], false, "response: {resp}");
    assert_eq!(resp["error"]["code"], "WRITE_RESULT_UNKNOWN");
    assert_eq!(resp["error"]["details"]["write_started"], true);
    assert_eq!(resp["error"]["details"]["cause_code"], "CDP_ERROR");

    let persisted =
        load_plan(&home, &root, "work", &plan.plan_id, Some("chat.send")).expect("load plan");
    assert_eq!(persisted.status, PlanStatus::Unknown);

    server.close().await;
}

/// Regression test for Finding 5: `ActiveRequestGuard` clears `active_request` on `Drop`
/// even if the executing task is abruptly aborted mid-flight.
#[tokio::test]
async fn active_request_guard_clears_active_request_on_abrupt_task_cancellation() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("home");
    let root = dir.path().join("project");
    create_profile(&home, &root, "work", None).expect("profile");

    let started = Arc::new(Notify::new());
    let started_ref = Arc::clone(&started);
    let handler = Arc::new(FnBrokerHandler::new(
        move |command, _params, _session, _home, _root, _profile| {
            let started_ref = Arc::clone(&started_ref);
            async move {
                if command == "hang.forever" {
                    started_ref.notify_one();
                    std::future::pending::<doudou_core::Result<Option<Value>>>().await
                } else {
                    Ok(None)
                }
            }
        },
    ));

    let config = BrokerServerConfig::new(home.clone(), root.clone(), "work")
        .with_fake_browser(true)
        .with_idle_timeout(Duration::from_secs(60));
    let server = Arc::new(
        BrokerServer::start(config, Some(handler))
            .await
            .expect("start broker"),
    );

    let srv = Arc::clone(&server);
    let handle = tokio::spawn(async move {
        srv.execute(&json!({
            "request_id": "aborted-req-1",
            "command": "hang.forever",
            "semantics": "read",
            "params": {},
        }))
        .await
    });

    started.notified().await;
    assert_eq!(server.active_request().as_deref(), Some("aborted-req-1"));
    assert!(server.is_command_locked());

    handle.abort();
    let _ = handle.await;

    assert_eq!(
        server.active_request(),
        None,
        "ActiveRequestGuard must clear active_request on task drop"
    );
    assert!(!server.is_command_locked());

    server.close().await;
}
