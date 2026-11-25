//! Chat feature services: conversations, message collection, send/reply/image/like plans & execution.

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use doudou_cdp::{AuthPolicy, ChromeSession};
use doudou_core::{
    DoudouError, Result, absolutize_path, default_local_message_window, expand_tilde,
    infer_self_uid, resolve_conversation, resolve_message, sha256_bytes,
};
use doudou_protocol::property_has_user;
use doudou_storage::{
    DEFAULT_PLAN_TTL_SECONDS, ReceiptKind, TerminalPlanStatus, WritePlan, complete_plan,
    confirm_plan, create_plan, load_plan, read_messages, write_receipt,
};
use doudou_ui::{like_message, send_message};
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

use crate::cdp_ui_bridge::CdpUiSession;
use crate::reads::chat::{collect_messages, list_conversations, refresh_session_index};

/// RAII drop guard ensuring an [`doudou_storage::PlanStatus::Executing`] plan is transitioned
/// to [`TerminalPlanStatus::Unknown`] if the enclosing async future is cancelled/dropped mid-flight.
#[derive(Debug)]
pub struct PlanExecutionGuard {
    home: PathBuf,
    data_root: PathBuf,
    profile: String,
    plan_id: String,
    active: bool,
}

impl PlanExecutionGuard {
    /// Activate the guard for a plan that has just been confirmed into `Executing`.
    pub fn new(home: &Path, data_root: &Path, profile: &str, plan_id: &str) -> Self {
        Self {
            home: home.to_path_buf(),
            data_root: data_root.to_path_buf(),
            profile: profile.to_owned(),
            plan_id: plan_id.to_owned(),
            active: true,
        }
    }

    /// Disarm the guard after explicitly completing the plan.
    pub fn defuse(&mut self) {
        self.active = false;
    }
}

impl Drop for PlanExecutionGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = complete_plan(
                &self.home,
                &self.data_root,
                &self.profile,
                &self.plan_id,
                TerminalPlanStatus::Unknown,
                None,
            );
        }
    }
}

fn value_as_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Verify that a sent message appears with matching ID, type, text, and reference in protocol read-back.
pub fn verify_send_readback(payload: &Value, result: &Value, messages: &[Value]) -> Value {
    let server_id = value_as_string(result.pointer("/verification/server_message_id"));
    let matches: Vec<&Value> = messages
        .iter()
        .filter(|item| value_as_string(item.get("message_id")) == server_id)
        .collect();
    let message = (matches.len() == 1).then(|| matches[0]);
    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("text");
    let expected_type: u64 = if mode == "image" { 27 } else { 7 };

    let text_match = message.is_some_and(|msg| {
        mode == "image"
            || msg
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .contains(payload.get("text").and_then(Value::as_str).unwrap_or(""))
    });
    let reply_obj = payload.get("reply").filter(|v| !v.is_null());
    let reference_match = message.is_some_and(|msg| match reply_obj {
        None => true,
        Some(reply) => {
            value_as_string(msg.get("reply_to_message_id"))
                == value_as_string(reply.get("message_id"))
        }
    });
    let msg_type_match = message
        .is_some_and(|msg| msg.get("message_type").and_then(Value::as_u64) == Some(expected_type));
    let conv_match = message.is_some_and(|msg| {
        value_as_string(msg.get("conversation_id"))
            == value_as_string(payload.pointer("/conversation/conversation_id"))
    });
    let passed = !server_id.is_empty()
        && matches.len() == 1
        && conv_match
        && msg_type_match
        && text_match
        && reference_match;

    json!({
        "passed": passed,
        "server_message_id": server_id,
        "matches": matches.len(),
        "message_type_match": msg_type_match,
        "text_match": text_match,
        "reference_match": reference_match,
    })
}

/// Verify that a liked chat message shows `self_uid` in its protocol `property_list`.
pub fn verify_like_readback(payload: &Value, messages: &[Value]) -> Value {
    let message_id = value_as_string(payload.pointer("/message/message_id"));
    let self_uid = value_as_string(payload.get("self_uid"));
    let matches: Vec<&Value> = messages
        .iter()
        .filter(|item| value_as_string(item.get("message_id")) == message_id)
        .collect();
    let liked = matches.len() == 1
        && matches[0]
            .get("property_list")
            .is_some_and(|props| property_has_user(Some(props), &self_uid));
    json!({
        "passed": liked,
        "message_id": message_id,
        "matches": matches.len(),
        "self_uid": self_uid,
        "user_liked": liked,
    })
}

pub(crate) async fn collect_readback(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    conversation: &Value,
    since_ms: Option<i64>,
) -> Result<(Value, Vec<Value>)> {
    let now = Utc::now();
    let lower = match since_ms {
        Some(ms) if ms > 0 => DateTime::from_timestamp_millis(ms).map_or_else(
            || now - ChronoDuration::minutes(10),
            |dt| dt - ChronoDuration::minutes(1),
        ),
        _ => now - ChronoDuration::minutes(10),
    };
    let upper = now + ChronoDuration::minutes(2);
    let conv_id = value_as_string(conversation.get("conversation_id"));
    let run = {
        let mut guard = session.lock().await;
        collect_messages(
            &mut guard,
            home,
            data_root,
            profile,
            &conv_id,
            Some(&lower.to_rfc3339()),
            Some(&upper.to_rfc3339()),
            10,
            false,
        )
        .await?
    };
    let stored = read_messages(home, data_root, profile, conversation)?;
    Ok((run, stored))
}

/// List conversations filtered by `kind` (`all|direct|group`) and `inbox` (`all|established|request`).
pub async fn conversations(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    force: bool,
    kind: &str,
    inbox: &str,
) -> Result<Value> {
    let mut guard = session.lock().await;
    list_conversations(&mut guard, home, data_root, profile, force, kind, inbox).await
}

/// Collect messages for `params["target"]`, defaulting to the local-midnight-to-now window.
pub async fn collect(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    params: &Value,
) -> Result<Value> {
    let target = params
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("INVALID_PARAMS", "Missing target conversation."))?;
    let mut from_time = params
        .get("from")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let mut to_time = params
        .get("to")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if from_time.is_none() && to_time.is_none() {
        let (start, end) = default_local_message_window();
        from_time = Some(start);
        to_time = Some(end);
    }
    let max_pages = params
        .get("max_pages")
        .and_then(Value::as_u64)
        .map_or(100, |v| v as usize);
    let download_images = !params
        .get("skip_images")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut result = {
        let mut guard = session.lock().await;
        collect_messages(
            &mut guard,
            home,
            data_root,
            profile,
            target,
            from_time.as_deref(),
            to_time.as_deref(),
            max_pages,
            download_images,
        )
        .await?
    };
    if let Some(obj) = result.as_object_mut() {
        obj.insert(
            "requested_window".to_owned(),
            json!({ "from": from_time, "to": to_time }),
        );
    }
    Ok(result)
}

/// Create a confirmation-required write plan for sending a chat text, reply, or image message.
pub async fn plan_send(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    params: &Value,
) -> Result<Value> {
    if params
        .get("at")
        .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
    {
        return Err(DoudouError::new(
            "MENTION_UNSUPPORTED",
            "Group mentions are not supported.",
        ));
    }
    let refreshed = {
        let mut guard = session.lock().await;
        refresh_session_index(&mut guard, home, data_root, profile, false).await?
    };
    let convs = refreshed
        .pointer("/index/conversations")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let target = params
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("INVALID_PARAMS", "Missing target conversation."))?;
    let conversation = resolve_conversation(convs, target)?;
    plan_send_for_conversation(home, data_root, profile, &conversation, params)
}

/// Create a send plan given an already-resolved `conversation` record (useful for direct testing).
pub fn plan_send_for_conversation(
    home: &Path,
    data_root: &Path,
    profile: &str,
    conversation: &Value,
    params: &Value,
) -> Result<Value> {
    if conversation.get("inbox").and_then(Value::as_str) == Some("request") {
        return Err(DoudouError::new(
            "STRANGER_WRITE_UNSUPPORTED",
            "Message requests are read-only and cannot be used for send plans.",
        ));
    }
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let image = params
        .get("image")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if text.is_some() == image.is_some() {
        return Err(DoudouError::new(
            "SEND_PAYLOAD_INVALID",
            "Exactly one of text or image is required.",
        ));
    }

    let mut payload = json!({
        "conversation": conversation,
        "mode": if image.is_some() { "image" } else { "text" },
    });

    if let Some(image_raw) = image {
        let source = absolutize_path(&expand_tilde(Path::new(image_raw)));
        if !source.is_file() {
            return Err(
                DoudouError::new("IMAGE_NOT_FOUND", "The image file does not exist.")
                    .with_details(json!({ "path": source.display().to_string() })),
            );
        }
        let bytes = fs::read(&source).map_err(|err| {
            DoudouError::new(
                "IO_ERROR",
                format!("Failed reading image {}: {err}", source.display()),
            )
        })?;
        let name = source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image");
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("image_path".to_owned(), json!(source.display().to_string()));
            obj.insert(
                "image".to_owned(),
                json!({
                    "name": name,
                    "bytes": bytes.len(),
                    "sha256": sha256_bytes(&bytes),
                }),
            );
        }
    } else if let Some(text_str) = text {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("text".to_owned(), json!(text_str));
        }
        if let Some(reply_query) = params
            .get("reply")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            let messages = read_messages(home, data_root, profile, conversation)?;
            let target_msg = resolve_message(&messages, reply_query)?;
            let target_text = target_msg
                .get("text")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    DoudouError::new(
                        "REPLY_TARGET_TEXT_MISSING",
                        "The page writer needs the collected target text to select the reply action.",
                    )
                })?;
            if let Some(obj) = payload.as_object_mut() {
                obj.insert(
                    "reply".to_owned(),
                    json!({
                        "message_id": target_msg.get("message_id").cloned().unwrap_or(Value::Null),
                        "text": target_text,
                        "sender": target_msg.get("sender").cloned().unwrap_or(Value::Null),
                    }),
                );
                obj.insert("mode".to_owned(), json!("reply"));
            }
        }
    }

    let plan = create_plan(
        home,
        data_root,
        profile,
        "chat.send",
        payload,
        DEFAULT_PLAN_TTL_SECONDS,
    )?;
    Ok(json!({
        "status": "confirmation_required",
        "plan": plan,
    }))
}

/// Execute a confirmed `chat.send` plan with an injectable send+readback runner.
///
/// Enforces the strict single-use `confirm_plan -> Executing -> complete_plan(Sent|Failed|Unknown)`
/// invariant with a [`PlanExecutionGuard`] so cancellation or mid-flight crash never allows replay.
pub async fn execute_send_with<F, Fut>(
    home: &Path,
    data_root: &Path,
    profile: &str,
    plan_id: &str,
    runner: F,
) -> Result<Value>
where
    F: FnOnce(WritePlan) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let stored = load_plan(home, data_root, profile, plan_id, Some("chat.send"))?;
    if stored
        .payload
        .get("mention")
        .is_some_and(|v| !v.is_null() && v.as_bool() != Some(false))
    {
        return Err(DoudouError::new(
            "MENTION_UNSUPPORTED",
            "This stored plan uses a group mention and can no longer be executed.",
        ));
    }
    let confirmed = confirm_plan(
        home,
        data_root,
        profile,
        plan_id,
        "chat.send",
        &stored.payload,
    )?;
    let mut guard = PlanExecutionGuard::new(home, data_root, profile, &confirmed.plan_id);

    match runner(confirmed.clone()).await {
        Ok(mut result) => {
            let mut receipt_record = result.as_object().cloned().unwrap_or_default();
            receipt_record.insert("plan_id".to_owned(), json!(confirmed.plan_id));
            let files = write_receipt(
                home,
                data_root,
                profile,
                "chat",
                "chat.send",
                &Value::Object(receipt_record),
                ReceiptKind::Send,
            )
            .map_err(DoudouError::mark_write_started)?;
            let receipt_rel = files.get("receipt").and_then(Value::as_str);
            let _ = complete_plan(
                home,
                data_root,
                profile,
                &confirmed.plan_id,
                TerminalPlanStatus::Sent,
                receipt_rel,
            )
            .map_err(DoudouError::mark_write_started)?;
            guard.defuse();
            if let Some(obj) = result.as_object_mut() {
                obj.insert("files".to_owned(), files);
            }
            Ok(result)
        }
        Err(err) => {
            let terminal = if err.is_write_started() {
                TerminalPlanStatus::Unknown
            } else {
                TerminalPlanStatus::Failed
            };
            let _ = complete_plan(home, data_root, profile, &confirmed.plan_id, terminal, None);
            guard.defuse();
            Err(err)
        }
    }
}

pub(crate) async fn run_send_readback<ReadbackFut>(
    plan_id: Option<&str>,
    payload: &Value,
    mut result: Value,
    readback_fut: ReadbackFut,
) -> Result<Value>
where
    ReadbackFut: Future<Output = Result<(Value, Vec<Value>)>>,
{
    let (read_run, messages) = readback_fut
        .await
        .map_err(DoudouError::mark_write_started)?;
    let mut readback = verify_send_readback(payload, &result, &messages);
    if let Some(rb_obj) = readback.as_object_mut() {
        rb_obj.insert(
            "collect_run_id".to_owned(),
            read_run.get("run_id").cloned().unwrap_or(Value::Null),
        );
    }
    let passed = readback
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(obj) = result.as_object_mut() {
        let ver = obj
            .entry("verification".to_owned())
            .or_insert_with(|| json!({}));
        if let Some(ver_obj) = ver.as_object_mut() {
            ver_obj.insert("protocol_recheck".to_owned(), readback.clone());
        }
    }
    if !passed {
        let mut details = serde_json::Map::new();
        if let Some(pid) = plan_id {
            details.insert("plan_id".to_owned(), json!(pid));
        }
        details.insert("write_started".to_owned(), json!(true));
        details.insert("protocol_recheck".to_owned(), readback);
        return Err(DoudouError::new(
            "WRITE_RESULT_UNKNOWN",
            "The page send succeeded but protocol read-back did not confirm it.",
        )
        .with_details(Value::Object(details)));
    }
    Ok(result)
}

/// Execute a confirmed `chat.send` write plan with injectable send and protocol read-back hooks.
pub async fn execute_send_with_readback<SendFn, SendFut, ReadbackFn, ReadbackFut>(
    home: &Path,
    data_root: &Path,
    profile: &str,
    plan_id: &str,
    send_fn: SendFn,
    readback_fn: ReadbackFn,
) -> Result<Value>
where
    SendFn: FnOnce(WritePlan) -> SendFut,
    SendFut: Future<Output = Result<Value>>,
    ReadbackFn: FnOnce(WritePlan) -> ReadbackFut,
    ReadbackFut: Future<Output = Result<(Value, Vec<Value>)>>,
{
    execute_send_with(home, data_root, profile, plan_id, move |plan| async move {
        let result = send_fn(plan.clone()).await?;
        let plan_id = plan.plan_id.clone();
        let payload = plan.payload.clone();
        run_send_readback(Some(&plan_id), &payload, result, readback_fn(plan)).await
    })
    .await
}

/// Execute a confirmed `chat.send` write plan through the live UI bridge and protocol read-back.
pub async fn execute_send(
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
    let send_session = Arc::clone(session);
    let readback_session = Arc::clone(session);
    let home_buf = home.to_path_buf();
    let root_buf = data_root.to_path_buf();
    let profile_str = profile.to_owned();

    execute_send_with_readback(
        home,
        data_root,
        profile,
        plan_id,
        move |plan| async move {
            let ui_session = CdpUiSession::from_session(send_session, AuthPolicy::FailFast).await?;
            let plan_json = serde_json::to_value(&plan).unwrap_or(Value::Null);
            send_message(&ui_session, &plan_json).await
        },
        move |plan| async move {
            collect_readback(
                &readback_session,
                &home_buf,
                &root_buf,
                &profile_str,
                &plan.payload["conversation"],
                None,
            )
            .await
        },
    )
    .await
}

/// Create a confirmation-required write plan for liking a collected chat message.
pub async fn plan_like(
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
    let target = params
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("INVALID_PARAMS", "Missing target conversation."))?;
    let conversation = resolve_conversation(convs, target)?;
    plan_like_for_conversation(home, data_root, profile, convs, &conversation, params)
}

/// Create a message-like plan given an already-resolved conversation index and target conversation.
pub fn plan_like_for_conversation(
    home: &Path,
    data_root: &Path,
    profile: &str,
    all_conversations: &[Value],
    conversation: &Value,
    params: &Value,
) -> Result<Value> {
    if conversation.get("inbox").and_then(Value::as_str) == Some("request") {
        return Err(DoudouError::new(
            "STRANGER_WRITE_UNSUPPORTED",
            "Message requests are read-only and cannot be used for like plans.",
        ));
    }
    let query = params
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("INVALID_PARAMS", "Missing message selector."))?;
    let messages = read_messages(home, data_root, profile, conversation)?;
    let message = resolve_message(&messages, query)?;
    let self_uid = infer_self_uid(all_conversations).ok_or_else(|| {
        DoudouError::new(
            "SELF_IDENTITY_UNRESOLVED",
            "The current account UID could not be derived from the IM protocol index.",
        )
    })?;
    let already_liked = property_has_user(message.get("property_list"), &self_uid);

    let plan = create_plan(
        home,
        data_root,
        profile,
        "chat.message.like",
        json!({
            "conversation": conversation,
            "message": {
                "message_id": message.get("message_id").cloned().unwrap_or(Value::Null),
                "text": message.get("text").cloned().unwrap_or(Value::Null),
                "property_list": message.get("property_list").cloned().unwrap_or(Value::Null),
                "create_time_ms": message.get("create_time_ms").cloned().unwrap_or(Value::Null),
            },
            "self_uid": self_uid,
            "already_liked": already_liked,
        }),
        DEFAULT_PLAN_TTL_SECONDS,
    )?;
    Ok(json!({
        "status": "confirmation_required",
        "plan": plan,
    }))
}

/// Execute a confirmed `chat.message.like` write plan with injectable UI like and read-back hooks.
pub async fn execute_like_with_readback<LikeFn, LikeFut, ReadbackFn, ReadbackFut>(
    home: &Path,
    data_root: &Path,
    profile: &str,
    plan_id: &str,
    like_fn: LikeFn,
    readback_fn: ReadbackFn,
) -> Result<Value>
where
    LikeFn: FnOnce(WritePlan) -> LikeFut,
    LikeFut: Future<Output = Result<Value>>,
    ReadbackFn: FnOnce(WritePlan, Option<i64>) -> ReadbackFut,
    ReadbackFut: Future<Output = Result<(Value, Vec<Value>)>>,
{
    let stored = load_plan(home, data_root, profile, plan_id, Some("chat.message.like"))?;
    let confirmed = confirm_plan(
        home,
        data_root,
        profile,
        plan_id,
        "chat.message.like",
        &stored.payload,
    )?;
    let mut guard = PlanExecutionGuard::new(home, data_root, profile, &confirmed.plan_id);

    let outcome: Result<Value> = async {
        let mut result = like_fn(confirmed.clone()).await?;
        let write_started = result.get("status").and_then(Value::as_str) != Some("already_liked");
        let since_ms = confirmed
            .payload
            .pointer("/message/create_time_ms")
            .and_then(Value::as_i64)
            .filter(|v| *v > 0);
        let (read_run, messages) =
            readback_fn(confirmed.clone(), since_ms)
                .await
                .map_err(|err| {
                    if write_started {
                        err.mark_write_started()
                    } else {
                        err
                    }
                })?;
        let mut readback = verify_like_readback(&confirmed.payload, &messages);
        if let Some(rb_obj) = readback.as_object_mut() {
            rb_obj.insert(
                "collect_run_id".to_owned(),
                read_run.get("run_id").cloned().unwrap_or(Value::Null),
            );
        }
        let passed = readback
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(obj) = result.as_object_mut() {
            let ver = obj
                .entry("verification".to_owned())
                .or_insert_with(|| json!({}));
            if let Some(ver_obj) = ver.as_object_mut() {
                ver_obj.insert("protocol_recheck".to_owned(), readback.clone());
                ver_obj.insert("passed".to_owned(), json!(passed));
                ver_obj.insert("pending_protocol_recheck".to_owned(), json!(false));
            }
        }
        if !passed {
            return Err(DoudouError::new(
                "WRITE_RESULT_UNKNOWN",
                "The message-like action was not confirmed by protocol read-back.",
            )
            .with_details(json!({
                "plan_id": confirmed.plan_id,
                "write_started": write_started,
                "protocol_recheck": readback,
            })));
        }
        Ok(result)
    }
    .await;

    match outcome {
        Ok(mut result) => {
            let write_started =
                result.get("status").and_then(Value::as_str) != Some("already_liked");
            let mut receipt_record = result.as_object().cloned().unwrap_or_default();
            receipt_record.insert("plan_id".to_owned(), json!(confirmed.plan_id));
            let files = write_receipt(
                home,
                data_root,
                profile,
                "chat",
                "chat.message.like",
                &Value::Object(receipt_record),
                ReceiptKind::Interaction,
            )
            .map_err(|err| {
                if write_started {
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
                &confirmed.plan_id,
                TerminalPlanStatus::Sent,
                receipt_rel,
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
            let terminal = if err.is_write_started() {
                TerminalPlanStatus::Unknown
            } else {
                TerminalPlanStatus::Failed
            };
            let _ = complete_plan(home, data_root, profile, &confirmed.plan_id, terminal, None);
            guard.defuse();
            Err(err)
        }
    }
}

/// Execute a confirmed `chat.message.like` write plan with protocol read-back verification.
pub async fn execute_like(
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
    let like_session = Arc::clone(session);
    let readback_session = Arc::clone(session);
    let home_buf = home.to_path_buf();
    let root_buf = data_root.to_path_buf();
    let profile_str = profile.to_owned();

    execute_like_with_readback(
        home,
        data_root,
        profile,
        plan_id,
        move |confirmed| async move {
            let ui_session = CdpUiSession::from_session(like_session, AuthPolicy::FailFast).await?;
            let plan_json = serde_json::to_value(&confirmed).unwrap_or(Value::Null);
            like_message(&ui_session, &plan_json).await
        },
        move |confirmed, since_ms| async move {
            collect_readback(
                &readback_session,
                &home_buf,
                &root_buf,
                &profile_str,
                &confirmed.payload["conversation"],
                since_ms,
            )
            .await
        },
    )
    .await
}
