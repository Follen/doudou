//! Video feature services: identity lookup, user videos, video/comment collection,
//! and confirmed comment create / reply / like writes with read-back verification.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use doudou_cdp::{AuthPolicy, ChromeSession};
use doudou_core::{DoudouError, IdentityCache, Result};
use doudou_storage::{
    DEFAULT_PLAN_TTL_SECONDS, ReceiptKind, TerminalPlanStatus, WritePlan, complete_plan,
    confirm_plan, create_plan, load_plan, read_video_comments, write_receipt,
};
use doudou_ui::{comment_write, like_comment};
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

use crate::cdp_ui_bridge::CdpUiSession;
use crate::features::chat::service::PlanExecutionGuard;
use crate::reads::video::{collect_user_videos, collect_video, resolve_identity};

fn value_as_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Verify a video interaction (`video.comment.create`, `video.comment.reply`, or
/// `video.comment.like`) against collected protocol comments.
pub fn verify_interaction_readback(
    operation: &str,
    payload: &Value,
    result: &Value,
    comments: &[Value],
) -> Value {
    if operation == "video.comment.like" {
        let expected_cid = value_as_string(payload.pointer("/comment/cid"));
        let matches: Vec<&Value> = comments
            .iter()
            .filter(|item| value_as_string(item.get("cid")) == expected_cid)
            .collect();
        let user_digged = matches
            .first()
            .and_then(|m| m.get("user_digged").and_then(Value::as_bool));
        let passed = matches.len() == 1 && user_digged == Some(true);
        return json!({
            "passed": passed,
            "operation": operation,
            "comment_id": expected_cid,
            "matches": matches.len(),
            "user_digged": user_digged,
        });
    }

    let expected_cid = value_as_string(result.pointer("/created_comment/cid"));
    let matches: Vec<&Value> = comments
        .iter()
        .filter(|item| value_as_string(item.get("cid")) == expected_cid)
        .collect();
    let expected_root = if operation == "video.comment.reply" {
        value_as_string(payload.pointer("/comment/cid"))
    } else {
        expected_cid.clone()
    };
    let expected_text = payload.get("text").and_then(Value::as_str).unwrap_or("");
    let text_match = matches
        .first()
        .is_some_and(|m| m.get("text").and_then(Value::as_str).unwrap_or("") == expected_text);
    let root_match = matches
        .first()
        .is_some_and(|m| value_as_string(m.get("reply_id")) == expected_root);
    let passed = !expected_cid.is_empty() && matches.len() == 1 && text_match && root_match;

    json!({
        "passed": passed,
        "operation": operation,
        "comment_id": expected_cid,
        "root_comment_id": expected_root,
        "matches": matches.len(),
        "text_match": text_match,
        "root_match": root_match,
    })
}

fn stored_protocol_recheck(
    home: &Path,
    data_root: &Path,
    profile: &str,
    operation: &str,
    plan: &WritePlan,
    result: &Value,
) -> Result<Value> {
    let aweme_id = value_as_string(plan.payload.get("aweme_id"));
    let comments = read_video_comments(home, data_root, profile, &aweme_id)?;
    let mut verification = verify_interaction_readback(operation, &plan.payload, result, &comments);
    let passed = verification
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(obj) = verification.as_object_mut() {
        obj.insert("authoritative".to_owned(), json!(false));
        obj.insert("source".to_owned(), json!("stored_protocol_snapshot"));
        obj.insert("collected_comments".to_owned(), json!(comments.len()));
        obj.insert(
            "reason".to_owned(),
            if passed {
                Value::Null
            } else {
                json!("stored_protocol_snapshot_did_not_include_the_new_comment")
            },
        );
    }
    Ok(verification)
}

async fn live_protocol_recheck(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    operation: &str,
    plan: &WritePlan,
    result: &Value,
) -> Result<Value> {
    let sec_uid = value_as_string(plan.payload.get("sec_uid"));
    let aweme_id = value_as_string(plan.payload.get("aweme_id"));
    let collected = {
        let mut guard = session.lock().await;
        collect_video(
            &mut guard, home, data_root, profile, &sec_uid, &aweme_id, 1000,
        )
        .await?
    };
    let comments = read_video_comments(home, data_root, profile, &aweme_id)?;
    let mut verification = verify_interaction_readback(operation, &plan.payload, result, &comments);
    if let Some(obj) = verification.as_object_mut() {
        obj.insert(
            "collected_comments".to_owned(),
            json!(
                collected
                    .get("comments")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            ),
        );
    }
    Ok(verification)
}

/// Resolve a Douyin ID to structured identity metadata.
pub async fn identity(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    douyin_id: &str,
) -> Result<Value> {
    let mut guard = session.lock().await;
    resolve_identity(&mut guard, home, data_root, profile, douyin_id).await
}

/// Collect a user's videos using cached or freshly resolved identity.
pub async fn videos(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    params: &Value,
) -> Result<Value> {
    let douyin_id = params
        .get("douyin_id")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("INVALID_PARAMS", "Missing douyin_id."))?;
    let max_pages = params
        .get("max_pages")
        .and_then(Value::as_u64)
        .map_or(1000, |v| v as usize);

    let cache = IdentityCache::load(home, data_root, profile)?;
    let resolved_identity = if let Some(cached) = cache.exact_unique_id(douyin_id) {
        cached
    } else {
        let mut guard = session.lock().await;
        let res = resolve_identity(&mut guard, home, data_root, profile, douyin_id).await?;
        res.get("identity").cloned().unwrap_or(Value::Null)
    };
    let mut guard = session.lock().await;
    collect_user_videos(
        &mut guard,
        home,
        data_root,
        profile,
        &resolved_identity,
        max_pages,
    )
    .await
}

/// Collect a single video's detail and comment/reply tree.
pub async fn video(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    params: &Value,
) -> Result<Value> {
    let sec_uid = params
        .get("sec_uid")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("VIDEO_TARGET_REQUIRED", "Missing sec_uid."))?;
    let aweme_id = params
        .get("aweme_id")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("VIDEO_TARGET_REQUIRED", "Missing aweme_id."))?;
    let max_pages = params
        .get("max_pages")
        .and_then(Value::as_u64)
        .map_or(1000, |v| v as usize);
    let mut guard = session.lock().await;
    collect_video(
        &mut guard, home, data_root, profile, sec_uid, aweme_id, max_pages,
    )
    .await
}

/// Create a confirmation-required write plan for a video comment create, reply, or like.
pub fn preview_interaction(
    home: &Path,
    data_root: &Path,
    profile: &str,
    operation: &str,
    payload: &Value,
) -> Result<Value> {
    let aweme_id = payload
        .get("aweme_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let sec_uid = payload
        .get("sec_uid")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let (Some(aweme_id), Some(_sec_uid)) = (aweme_id, sec_uid) else {
        return Err(DoudouError::new(
            "VIDEO_TARGET_REQUIRED",
            "A resolved sec_uid and aweme_id are required.",
        ));
    };

    if matches!(operation, "video.comment.create" | "video.comment.reply")
        && payload
            .get("text")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(DoudouError::new(
            "COMMENT_TEXT_REQUIRED",
            "A non-empty confirmed comment body is required.",
        ));
    }

    let mut final_payload = payload.clone();
    if matches!(operation, "video.comment.reply" | "video.comment.like") {
        let comment_id = value_as_string(payload.pointer("/comment/cid"));
        let all_comments = read_video_comments(home, data_root, profile, aweme_id)?;
        let matches: Vec<&Value> = all_comments
            .iter()
            .filter(|item| value_as_string(item.get("cid")) == comment_id)
            .collect();
        if matches.len() != 1 {
            return Err(DoudouError::new(
                "COMMENT_TARGET_NOT_FOUND",
                "The comment ID is not unique in the collected protocol records.",
            )
            .with_details(json!({
                "comment_id": comment_id,
                "matches": matches.len(),
            })));
        }
        let supplied_text = payload
            .pointer("/comment/text")
            .and_then(Value::as_str)
            .unwrap_or("");
        let actual_text = matches[0].get("text").and_then(Value::as_str).unwrap_or("");
        if !supplied_text.is_empty() && supplied_text != actual_text {
            return Err(DoudouError::new(
                "COMMENT_TARGET_DRIFTED",
                "The supplied comment text does not match the collected protocol record.",
            )
            .with_details(json!({ "comment_id": comment_id })));
        }
        if let Some(obj) = final_payload.as_object_mut() {
            obj.insert("comment".to_owned(), matches[0].clone());
        }
    }

    let plan = create_plan(
        home,
        data_root,
        profile,
        operation,
        final_payload,
        DEFAULT_PLAN_TTL_SECONDS,
    )?;
    Ok(json!({
        "status": "confirmation_required",
        "plan": plan,
    }))
}

/// Plan a top-level video comment creation (`video.comment.create`).
pub fn plan_comment(
    home: &Path,
    data_root: &Path,
    profile: &str,
    payload: &Value,
) -> Result<Value> {
    preview_interaction(home, data_root, profile, "video.comment.create", payload)
}

/// Plan a reply to an existing collected video comment (`video.comment.reply`).
pub fn plan_reply(home: &Path, data_root: &Path, profile: &str, payload: &Value) -> Result<Value> {
    preview_interaction(home, data_root, profile, "video.comment.reply", payload)
}

/// Plan liking an existing collected video comment (`video.comment.like`).
pub fn plan_comment_like(
    home: &Path,
    data_root: &Path,
    profile: &str,
    payload: &Value,
) -> Result<Value> {
    preview_interaction(home, data_root, profile, "video.comment.like", payload)
}

/// Execute a confirmed video interaction write plan with injectable write and protocol recheck hooks.
pub async fn execute_interaction_with<WriteFn, WriteFut, RecheckFn, RecheckFut>(
    home: &Path,
    data_root: &Path,
    profile: &str,
    operation: &str,
    plan_id: &str,
    write_fn: WriteFn,
    recheck_fn: RecheckFn,
) -> Result<Value>
where
    WriteFn: FnOnce(WritePlan) -> WriteFut,
    WriteFut: Future<Output = Result<Value>>,
    RecheckFn: FnOnce(WritePlan, Value) -> RecheckFut,
    RecheckFut: Future<Output = Result<Value>>,
{
    let stored = load_plan(home, data_root, profile, plan_id, Some(operation))?;
    let plan = confirm_plan(
        home,
        data_root,
        profile,
        plan_id,
        operation,
        &stored.payload,
    )?;
    let mut guard = PlanExecutionGuard::new(home, data_root, profile, &plan.plan_id);

    let outcome: Result<Value> = async {
        let mut result = write_fn(plan.clone()).await?;
        let write_started = result.get("status").and_then(Value::as_str) != Some("already_liked");
        let readback = recheck_fn(plan.clone(), result.clone())
            .await
            .map_err(|err| {
                if write_started {
                    err.mark_write_started()
                } else {
                    err
                }
            })?;
        if let Some(obj) = result.as_object_mut() {
            let ver = obj
                .entry("verification".to_owned())
                .or_insert_with(|| json!({}));
            if let Some(ver_obj) = ver.as_object_mut() {
                ver_obj.insert("protocol_recheck".to_owned(), readback);
            }
        }
        Ok(result)
    }
    .await;

    match outcome {
        Ok(mut result) => {
            let write_started =
                result.get("status").and_then(Value::as_str) != Some("already_liked");
            let mut receipt_payload = result.as_object().cloned().unwrap_or_default();
            receipt_payload.insert("plan_id".to_owned(), json!(plan.plan_id));
            let files = write_receipt(
                home,
                data_root,
                profile,
                "video",
                operation,
                &Value::Object(receipt_payload),
                ReceiptKind::Interaction,
            )
            .map_err(|err| {
                if write_started {
                    err.mark_write_started()
                } else {
                    err
                }
            })?;
            let receipt_path = files.get("receipt").and_then(Value::as_str);
            let _ = complete_plan(
                home,
                data_root,
                profile,
                &plan.plan_id,
                TerminalPlanStatus::Sent,
                receipt_path,
            )
            .map_err(|err| {
                if write_started {
                    err.mark_write_started()
                } else {
                    err
                }
            })?;
            guard.defuse();
            if let Some(obj) = result.as_object_mut() {
                obj.insert("files".to_owned(), files);
            }
            Ok(result)
        }
        Err(err) => {
            let result_known = err
                .details
                .as_ref()
                .and_then(|d| d.get("result_known"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let terminal = if result_known || !err.is_write_started() {
                TerminalPlanStatus::Failed
            } else {
                TerminalPlanStatus::Unknown
            };
            let _ = complete_plan(home, data_root, profile, &plan.plan_id, terminal, None);
            guard.defuse();
            Err(err)
        }
    }
}

/// Execute a confirmed video interaction write plan with single-use confirmation and receipt logging.
pub async fn execute_interaction(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    operation: &str,
    params: &Value,
) -> Result<Value> {
    let plan_id = params
        .get("plan_id")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("PLAN_NOT_FOUND", "Missing plan_id."))?;
    let write_session = Arc::clone(session);
    let recheck_session = Arc::clone(session);
    let op_write = operation.to_owned();
    let op_recheck = operation.to_owned();
    let home_buf = home.to_path_buf();
    let root_buf = data_root.to_path_buf();
    let profile_str = profile.to_owned();

    execute_interaction_with(
        home,
        data_root,
        profile,
        operation,
        plan_id,
        move |plan| async move {
            let ui_session =
                CdpUiSession::from_session(write_session, AuthPolicy::FailFast).await?;
            let plan_json = serde_json::to_value(&plan).unwrap_or(Value::Null);
            if op_write == "video.comment.like" {
                like_comment(&ui_session, &plan_json).await
            } else {
                comment_write(&ui_session, &plan_json).await
            }
        },
        move |plan, result| async move {
            if matches!(
                op_recheck.as_str(),
                "video.comment.create" | "video.comment.reply"
            ) {
                return stored_protocol_recheck(
                    &home_buf,
                    &root_buf,
                    &profile_str,
                    &op_recheck,
                    &plan,
                    &result,
                );
            }
            let mut readback = live_protocol_recheck(
                &recheck_session,
                &home_buf,
                &root_buf,
                &profile_str,
                &op_recheck,
                &plan,
                &result,
            )
            .await?;
            let passed = readback
                .get("passed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(rb_obj) = readback.as_object_mut() {
                rb_obj.insert("authoritative".to_owned(), json!(false));
                rb_obj.insert("source".to_owned(), json!("ranked_comment_protocol"));
                rb_obj.insert(
                    "reason".to_owned(),
                    if passed {
                        Value::Null
                    } else {
                        json!("ranked_comment_protocol_did_not_confirm_account_like_state")
                    },
                );
            }
            Ok(readback)
        },
    )
    .await
}

/// Execute a confirmed `video.comment.create` write plan.
pub async fn execute_comment(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    params: &Value,
) -> Result<Value> {
    execute_interaction(
        session,
        home,
        data_root,
        profile,
        "video.comment.create",
        params,
    )
    .await
}

/// Execute a confirmed `video.comment.reply` write plan.
pub async fn execute_reply(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    params: &Value,
) -> Result<Value> {
    execute_interaction(
        session,
        home,
        data_root,
        profile,
        "video.comment.reply",
        params,
    )
    .await
}

/// Execute a confirmed `video.comment.like` write plan.
pub async fn execute_comment_like(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    params: &Value,
) -> Result<Value> {
    execute_interaction(
        session,
        home,
        data_root,
        profile,
        "video.comment.like",
        params,
    )
    .await
}
