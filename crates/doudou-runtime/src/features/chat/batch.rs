//! Sequential batch chat send planning and fail-closed execution.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use doudou_cdp::{AuthPolicy, ChromeSession};
use doudou_core::{DoudouError, Result, resolve_conversation};
use doudou_storage::{
    DEFAULT_PLAN_TTL_SECONDS, ReceiptKind, TerminalPlanStatus, complete_plan, confirm_plan,
    create_plan, load_plan, write_receipt,
};
use doudou_ui::send_message;
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

use crate::cdp_ui_bridge::CdpUiSession;
use crate::features::chat::service::{PlanExecutionGuard, collect_readback, run_send_readback};
use crate::reads::chat::refresh_session_index;

/// Minimum spacing interval in seconds between consecutive batch send attempts.
pub const MIN_BATCH_INTERVAL_SECONDS: f64 = 5.0;

/// Create a confirmation-required write plan for sequential batch chat text sends.
pub async fn plan_batch(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    params: &Value,
) -> Result<Value> {
    let refreshed = {
        let mut guard = session.lock().await;
        refresh_session_index(&mut guard, home, data_root, profile, false).await?
    };
    let convs = refreshed
        .pointer("/index/conversations")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    plan_batch_from_conversations(home, data_root, profile, convs, params)
}

/// Create a batch send plan given an already-loaded conversation list.
pub fn plan_batch_from_conversations(
    home: &Path,
    data_root: &Path,
    profile: &str,
    conversations: &[Value],
    params: &Value,
) -> Result<Value> {
    let targets: Vec<String> = params
        .get("targets")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let text = params.get("text").and_then(Value::as_str).unwrap_or("");
    if targets.is_empty() || text.is_empty() {
        return Err(DoudouError::new(
            "BATCH_PLAN_INVALID",
            "Batch send requires at least one target and non-empty text.",
        ));
    }

    let mut tasks = Vec::with_capacity(targets.len());
    for target in &targets {
        let conv = resolve_conversation(conversations, target)?;
        if conv.get("inbox").and_then(Value::as_str) == Some("request") {
            return Err(DoudouError::new(
                "STRANGER_WRITE_UNSUPPORTED",
                "Message requests are read-only and cannot be used for send plans.",
            ));
        }
        tasks.push(json!({
            "conversation": conv,
            "mode": "text",
            "text": text,
        }));
    }

    let plan = create_plan(
        home,
        data_root,
        profile,
        "chat.send.batch",
        json!({
            "tasks": tasks,
            "interval_seconds": MIN_BATCH_INTERVAL_SECONDS,
        }),
        DEFAULT_PLAN_TTL_SECONDS,
    )?;
    Ok(json!({
        "status": "confirmation_required",
        "plan": plan,
    }))
}

/// Execute a confirmed `chat.send.batch` plan using injectable per-item sender and sleeper hooks.
///
/// Enforces:
/// - Single-use `confirm_plan` -> `Executing` -> `complete_plan` with [`PlanExecutionGuard`]
/// - Minimum 5-second spacing between items
/// - Stops immediately on the first failed or unknown item and marks all remaining items `"skipped"`
/// - Preserves `FailedBeforeExternalSend`: if any earlier item already sent (`sent_count > 0`) and
///   a later item fails deterministically before send, the overall batch plan finalizes as
///   [`TerminalPlanStatus::Unknown`] (not `Failed`) while individual rows retain `"sent"`,
///   `"failed"`, and `"skipped"` statuses.
pub async fn execute_batch_with<SendFn, SendFut, SleepFn, SleepFut>(
    home: &Path,
    data_root: &Path,
    profile: &str,
    plan_id: &str,
    mut send_item: SendFn,
    mut sleep_fn: SleepFn,
) -> Result<Value>
where
    SendFn: FnMut(Value) -> SendFut,
    SendFut: Future<Output = Result<Value>>,
    SleepFn: FnMut(f64) -> SleepFut,
    SleepFut: Future<Output = ()>,
{
    let stored = load_plan(home, data_root, profile, plan_id, Some("chat.send.batch"))?;
    let plan = confirm_plan(
        home,
        data_root,
        profile,
        plan_id,
        "chat.send.batch",
        &stored.payload,
    )?;
    let mut guard = PlanExecutionGuard::new(home, data_root, profile, &plan.plan_id);

    let tasks = plan
        .payload
        .get("tasks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let interval_seconds = plan
        .payload
        .get("interval_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if interval_seconds < MIN_BATCH_INTERVAL_SECONDS {
        let _ = complete_plan(
            home,
            data_root,
            profile,
            &plan.plan_id,
            TerminalPlanStatus::Failed,
            None,
        );
        guard.defuse();
        return Err(DoudouError::new(
            "INVALID_SEND_INTERVAL",
            "Batch send interval must be at least five seconds.",
        ));
    }

    let mut results: Vec<Value> = Vec::with_capacity(tasks.len());
    let mut sent_count = 0usize;
    let mut failed = false;
    let mut unknown = false;

    for (index, payload) in tasks.iter().enumerate() {
        if failed || unknown {
            results.push(json!({ "index": index, "status": "skipped" }));
            continue;
        }
        let subplan = json!({
            "schema": "doudou.write-plan.v1",
            "plan_id": format!("{}:{index}", plan.plan_id),
            "operation": "chat.send",
            "payload": payload,
        });
        match send_item(subplan).await {
            Ok(item_result) => {
                sent_count += 1;
                let mut receipt_rec = item_result.as_object().cloned().unwrap_or_default();
                receipt_rec.insert("batch_plan_id".to_owned(), json!(plan.plan_id));
                receipt_rec.insert("task_index".to_owned(), json!(index));
                let files = write_receipt(
                    home,
                    data_root,
                    profile,
                    "chat",
                    "chat.send",
                    &Value::Object(receipt_rec),
                    ReceiptKind::Send,
                )
                .map_err(DoudouError::mark_write_started)?;
                let mut row = item_result.as_object().cloned().unwrap_or_default();
                row.insert("index".to_owned(), json!(index));
                row.insert("files".to_owned(), files);
                results.push(Value::Object(row));
            }
            Err(err) => {
                let is_unknown = err.code == "WRITE_RESULT_UNKNOWN" || err.is_write_started();
                if is_unknown {
                    unknown = true;
                } else {
                    failed = true;
                }
                results.push(json!({
                    "index": index,
                    "status": if is_unknown { "unknown" } else { "failed" },
                    "error_code": err.code,
                }));
                continue;
            }
        }
        if index + 1 < tasks.len() {
            sleep_fn(interval_seconds).await;
        }
    }

    let any_write_started = sent_count > 0 || unknown;
    let (status_str, terminal) = if unknown || (failed && sent_count > 0) {
        ("unknown", TerminalPlanStatus::Unknown)
    } else if failed {
        ("failed", TerminalPlanStatus::Failed)
    } else {
        ("sent", TerminalPlanStatus::Sent)
    };

    let files = write_receipt(
        home,
        data_root,
        profile,
        "chat",
        "chat.send.batch",
        &json!({
            "plan_id": plan.plan_id,
            "status": status_str,
            "results": results,
            "interval_seconds": interval_seconds,
        }),
        ReceiptKind::Send,
    )
    .map_err(|err| {
        if any_write_started {
            err.mark_write_started()
        } else {
            err
        }
    })?;
    let receipt_rel = files.get("receipt").and_then(Value::as_str);
    let _ = complete_plan(
        home,
        data_root,
        profile,
        &plan.plan_id,
        terminal,
        receipt_rel,
    )
    .map_err(|err| {
        if any_write_started {
            err.mark_write_started()
        } else {
            err
        }
    })?;
    guard.defuse();

    Ok(json!({
        "status": status_str,
        "plan_id": plan.plan_id,
        "results": results,
        "interval_seconds": interval_seconds,
        "files": files,
    }))
}

/// Execute a confirmed `chat.send.batch` write plan against the live browser session.
pub async fn execute_batch(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    params: &Value,
) -> Result<Value> {
    let plan_id = params
        .get("plan_id")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("PLAN_NOT_FOUND", "Missing plan_id."))?;

    execute_batch_with(
        home,
        data_root,
        profile,
        plan_id,
        |subplan| {
            let session = Arc::clone(session);
            let home_buf = home.to_path_buf();
            let root_buf = data_root.to_path_buf();
            let profile_str = profile.to_owned();
            async move {
                let ui_session =
                    CdpUiSession::from_session(Arc::clone(&session), AuthPolicy::FailFast).await?;
                let result = send_message(&ui_session, &subplan).await?;
                let payload = &subplan["payload"];
                run_send_readback(
                    None,
                    payload,
                    result,
                    collect_readback(
                        &session,
                        &home_buf,
                        &root_buf,
                        &profile_str,
                        &payload["conversation"],
                        None,
                    ),
                )
                .await
            }
        },
        |seconds| async move {
            tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
        },
    )
    .await
}
