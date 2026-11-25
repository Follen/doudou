//! High-level chat UI write flows: open conversation, send text/reply/image, and like message.

use std::path::Path;

use doudou_core::{DoudouError, Result, sha256_bytes, sha256_str};
use doudou_protocol::{parse_send_request, parse_send_response};
use serde_json::{Value, json};

use crate::components::{
    ChatComposerComponent, ConversationListComponent, ImagePreviewComponent, MessageItemComponent,
};
use crate::page::{CaptureFilter, NetworkCaptureSnapshot, UiPage, UiSession};

/// URL substrings captured during chat send flows.
pub const SEND_PATHS: [&str; 5] = [
    "/v1/message/send",
    "/aweme/v1/im/consistency/action/report",
    "/passport/safe/get_identity_security_token/",
    "/upload/",
    "batch_build_image",
];

fn value_as_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Ensure the session is ready, navigate to `/chat` if needed, and open `display_name`.
pub async fn open_conversation(session: &dyn UiSession, display_name: &str) -> Result<usize> {
    session.ensure_ready().await?;
    let current_url = session.page().url().await?;
    if !current_url.starts_with("https://www.douyin.com/chat") {
        session.open("https://www.douyin.com/chat").await?;
    }
    let operations = ConversationListComponent::new(session.page())
        .open(display_name)
        .await?;
    session.page().sleep_ms(1_200).await?;
    Ok(operations)
}

async fn select_reply(page: &dyn UiPage, target_text: &str) -> Result<usize> {
    let component = MessageItemComponent::new(page);
    let target = component.target(target_text, "chat.reply").await?;
    component.menu_action(&target, "回复", "chat.reply").await
}

/// Verify captured `/v1/message/send` Protobuf request and response against `plan`.
pub fn verify_send_capture(capture: &NetworkCaptureSnapshot, plan: &Value) -> Value {
    let requests: Vec<&[u8]> = capture
        .requests
        .iter()
        .filter(|item| item.path.contains("/v1/message/send"))
        .map(|item| item.body.as_slice())
        .collect();
    let responses: Vec<&[u8]> = capture
        .responses
        .iter()
        .filter(|item| item.path.contains("/v1/message/send"))
        .map(|item| item.body.as_slice())
        .collect();

    let payload = &plan["payload"];
    let mut request = Value::Object(serde_json::Map::new());
    let mut response = Value::Object(serde_json::Map::new());
    let mut parse_error: Option<String> = None;

    if requests.len() == 1 {
        match parse_send_request(requests[0]) {
            Ok(parsed) => request = parsed,
            Err(err) => parse_error = Some(err.code),
        }
    }
    if parse_error.is_none() && responses.len() == 1 {
        match parse_send_response(responses[0]) {
            Ok(parsed) => response = parsed,
            Err(err) => parse_error = Some(err.code),
        }
    }

    let content = request.get("content").and_then(Value::as_object);
    let actual_text = content.and_then(|c| c.get("text")).and_then(Value::as_str);
    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("text");
    let expected_text = if mode == "image" {
        None
    } else {
        payload.get("text").and_then(Value::as_str)
    };
    let conversation = &payload["conversation"];
    let expected_type: u64 = if mode == "image" { 27 } else { 7 };

    let image_shape_match = mode != "image"
        || content.is_some_and(|c| {
            let awe_ok = c.get("aweType").and_then(Value::as_u64) == Some(2702);
            let resource = c.get("resource_url").and_then(Value::as_object);
            let oid_ok = resource
                .and_then(|r| r.get("oid"))
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty());
            let skey_ok = resource
                .and_then(|r| r.get("skey"))
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty());
            awe_ok && oid_ok && skey_ok
        });

    let text_match = expected_text.is_none() || actual_text == expected_text;

    let expected_conv_id = value_as_string(conversation.get("conversation_id"));
    let expected_short_id = value_as_string(conversation.get("conversation_short_id"));
    let expected_conv_type = conversation
        .get("conversation_type")
        .and_then(Value::as_u64);

    let req_conv_id = request
        .get("conversation_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let req_short_id = request
        .get("conversation_short_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let req_conv_type = request.get("conversation_type").and_then(Value::as_u64);
    let req_msg_type = request.get("message_type").and_then(Value::as_u64);
    let req_client_id = request
        .get("client_message_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());

    let reply_obj = payload.get("reply").filter(|v| !v.is_null());
    let reference_match = match reply_obj {
        None => true,
        Some(reply) => {
            let expected_ref = value_as_string(reply.get("message_id"));
            request.get("referenced_message_id").and_then(Value::as_str)
                == Some(expected_ref.as_str())
        }
    };

    let conversation_match = req_conv_id == expected_conv_id;
    let message_type_match = req_msg_type == Some(expected_type);

    let request_matches = conversation_match
        && req_short_id == expected_short_id
        && req_conv_type == expected_conv_type
        && message_type_match
        && req_client_id.is_some()
        && text_match
        && image_shape_match
        && reference_match;

    let resp_client_id = response
        .get("client_message_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let client_id_match = req_client_id.is_some() && resp_client_id == req_client_id;
    let server_message_id = response
        .get("server_message_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let gateway = response.get("gateway").filter(|v| !v.is_null());
    let gateway_decision = gateway
        .and_then(|g| g.get("decision"))
        .and_then(Value::as_str);

    let passed = requests.len() == 1
        && responses.len() == 1
        && parse_error.is_none()
        && request_matches
        && response.get("protobuf_status").and_then(Value::as_u64) == Some(0)
        && response.get("business_status").and_then(Value::as_u64) == Some(0)
        && server_message_id.is_some()
        && client_id_match
        && gateway.is_none();

    json!({
        "passed": passed,
        "request_count": requests.len(),
        "response_count": responses.len(),
        "protobuf_status": response.get("protobuf_status").cloned().unwrap_or(Value::Null),
        "status_text": response.get("status_text").cloned().unwrap_or(Value::Null),
        "business_status": response.get("business_status").cloned().unwrap_or(Value::Null),
        "gateway_decision": gateway_decision,
        "parse_error": parse_error,
        "conversation_match": conversation_match,
        "message_type_match": message_type_match,
        "text_match": text_match,
        "reference_match": reference_match,
        "image_shape_match": image_shape_match,
        "server_message_id_present": server_message_id.is_some(),
        "server_message_id": server_message_id,
        "client_message_id": resp_client_id,
        "client_message_id_match": client_id_match,
        "request_sha256": (requests.len() == 1).then(|| sha256_bytes(requests[0])),
        "response_sha256": (responses.len() == 1).then(|| sha256_bytes(responses[0])),
    })
}

/// Execute a confirmed chat send/reply/image write plan and verify its Protobuf receipt.
pub async fn send_message(session: &dyn UiSession, plan: &Value) -> Result<Value> {
    let payload = &plan["payload"];
    let plan_id = value_as_string(plan.get("plan_id"));
    let display_name = payload["conversation"]["display_name"]
        .as_str()
        .ok_or_else(|| DoudouError::new("INVALID_PLAN", "Missing conversation.display_name"))?;
    let page = session.page();
    let mut dom_operations = open_conversation(session, display_name).await?;

    let capture_handle = page
        .start_network_capture(CaptureFilter::contains(SEND_PATHS))
        .await?;

    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("text");

    let mut write_started = false;
    let send_step: Result<()> = async {
        if mode == "image" {
            let image_path = payload
                .get("image_path")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    DoudouError::new("IMAGE_NOT_FOUND", "Missing image_path in plan.")
                })?;
            let source = Path::new(image_path);
            if !source.is_file() {
                return Err(DoudouError::new(
                    "IMAGE_NOT_FOUND",
                    "The confirmed image file no longer exists.",
                )
                .with_details(json!({ "path": image_path })));
            }
            let mut image_preview = ImagePreviewComponent::new(page);
            dom_operations += image_preview.choose(source).await?;
            page.sleep_ms(1_000).await?;
            dom_operations += image_preview.send().await?;
            write_started = true;
        } else {
            if let Some(reply_text) = payload
                .get("reply")
                .and_then(|r| r.get("text"))
                .and_then(Value::as_str)
            {
                dom_operations += select_reply(page, reply_text).await?;
            }
            let planned_text = payload
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| DoudouError::new("INVALID_PLAN", "Missing text in send plan."))?;
            let composer = ChatComposerComponent::new(page);
            let _ = composer.clear().await?;
            let _ = composer.insert_text(planned_text).await?;
            dom_operations += 2;
            let actual_text = composer.text().await?;
            if actual_text != planned_text {
                let _ = composer.clear().await;
                return Err(DoudouError::new(
                    "COMPOSER_PLAN_MISMATCH",
                    "The page composer does not exactly match the confirmed plan.",
                )
                .with_details(json!({
                    "planned_sha256": sha256_str(planned_text),
                    "actual_sha256": sha256_str(&actual_text),
                })));
            }
            match composer.send().await {
                Ok(ops) => {
                    dom_operations += ops;
                    write_started = true;
                }
                Err(_) => {
                    return Err(DoudouError::new(
                        "WRITE_RESULT_UNKNOWN",
                        "The chat send control was invoked but its result is uncertain.",
                    )
                    .with_details(json!({
                        "plan_id": plan_id,
                        "write_started": true,
                    })));
                }
            }
        }

        for _ in 0..150 {
            let snap = capture_handle
                .snapshot()
                .await
                .map_err(DoudouError::mark_write_started)?;
            if snap
                .responses
                .iter()
                .any(|resp| resp.path.contains("/v1/message/send"))
            {
                break;
            }
            page.sleep_ms(100)
                .await
                .map_err(DoudouError::mark_write_started)?;
        }
        Ok(())
    }
    .await;

    let write_started = write_started || matches!(&send_step, Err(err) if err.is_write_started());
    let capture = capture_handle.stop().await.map_err(|err| {
        if write_started {
            err.mark_write_started()
        } else {
            err
        }
    })?;
    send_step?;

    let verification = verify_send_capture(&capture, plan);
    let passed = verification
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !passed {
        let kicked = verification.get("gateway_decision").and_then(Value::as_str) == Some("KICK");
        if kicked
            || session
                .verification_visible()
                .await
                .map_err(DoudouError::mark_write_started)?
        {
            let kind = session
                .verification_kind()
                .await
                .map_err(DoudouError::mark_write_started)?;
            return Err(DoudouError::new(
                "WRITE_VERIFICATION_REQUIRED",
                "The page write triggered account verification and was not replayed.",
            )
            .with_details(json!({
                "plan_id": plan_id,
                "verification": verification,
                "verification_kind": kind,
                "write_started": true,
                "automatic_replay_allowed": false,
            })));
        }
        return Err(DoudouError::new(
            "WRITE_RESULT_UNKNOWN",
            "The page send did not produce one verified protocol receipt.",
        )
        .with_details(json!({
            "plan_id": plan_id,
            "verification": verification,
            "write_started": true,
        })));
    }

    Ok(json!({
        "status": "sent",
        "plan_id": plan_id,
        "dom_operations": dom_operations,
        "react_fiber_traversal": false,
        "network": capture.to_network_json(),
        "verification": verification,
    }))
}

/// Execute a confirmed chat message like write plan via the message context menu.
pub async fn like_message(session: &dyn UiSession, plan: &Value) -> Result<Value> {
    let payload = &plan["payload"];
    let plan_id = value_as_string(plan.get("plan_id"));
    if payload
        .get("already_liked")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(json!({
            "status": "already_liked",
            "plan_id": plan_id,
            "dom_operations": 0,
            "network": { "requests": [], "responses": [] },
            "verification": {
                "passed": true,
                "already_liked": true,
                "source": "protocol_plan",
            },
        }));
    }

    let display_name = payload["conversation"]["display_name"]
        .as_str()
        .ok_or_else(|| DoudouError::new("INVALID_PLAN", "Missing conversation.display_name"))?;
    let message_text = payload["message"]["text"]
        .as_str()
        .ok_or_else(|| DoudouError::new("INVALID_PLAN", "Missing message.text"))?;

    let page = session.page();
    let mut dom_operations = open_conversation(session, display_name).await?;
    let component = MessageItemComponent::new(page);
    let target = component.target(message_text, "chat.message.like").await?;

    let capture_handle = page
        .start_network_capture(CaptureFilter::contains([
            "/v1/message/modify_property",
            "/v1/message/set_property",
        ]))
        .await?;

    let mut write_started = false;
    let action_result: Result<()> = async {
        dom_operations += component.like_action(&target, "chat.message.like").await?;
        write_started = true;
        page.sleep_ms(3_000)
            .await
            .map_err(DoudouError::mark_write_started)?;
        Ok(())
    }
    .await;

    let write_started =
        write_started || matches!(&action_result, Err(err) if err.is_write_started());
    let capture = capture_handle.stop().await.map_err(|err| {
        if write_started {
            err.mark_write_started()
        } else {
            err
        }
    })?;
    action_result?;

    let is_property_endpoint = |path: &str| {
        let normalized = path.trim_end_matches('/');
        normalized == "/v1/message/modify_property" || normalized == "/v1/message/set_property"
    };
    let modify_requests: Vec<_> = capture
        .requests
        .iter()
        .filter(|item| is_property_endpoint(&item.endpoint.path))
        .collect();
    let modify_responses: Vec<_> = capture
        .responses
        .iter()
        .filter(|item| is_property_endpoint(&item.endpoint.path))
        .collect();

    let transport_observed = !modify_requests.is_empty() || !modify_responses.is_empty();
    let transport_consistent = !transport_observed
        || (modify_requests.len() == 1
            && modify_responses.len() == 1
            && modify_responses[0].http_status == 200);
    if !transport_consistent {
        return Err(DoudouError::new(
            "WRITE_RESULT_UNKNOWN",
            "The message-like transport observation was incomplete.",
        )
        .with_details(json!({
            "plan_id": plan_id,
            "write_started": true,
            "request_count": modify_requests.len(),
            "response_count": modify_responses.len(),
        })));
    }

    Ok(json!({
        "status": "liked",
        "plan_id": plan_id,
        "dom_operations": dom_operations,
        "network": capture.to_network_json(),
        "verification": {
            "passed": false,
            "pending_protocol_recheck": true,
            "transport_observed": transport_observed,
            "request_count": modify_requests.len(),
            "response_count": modify_responses.len(),
        },
    }))
}
