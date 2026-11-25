//! High-level video UI write flows: comment creation, reply, and comment like.

use std::collections::{BTreeMap, BTreeSet};

use doudou_core::{DoudouError, Result};
use doudou_protocol::{
    parse_page_write_response, request_fields, response_comment, validate_comment_request, web_path,
};
use serde_json::{Map, Value, json};

use crate::components::{
    CommentItemComponent, VideoCommentComposerComponent, VideoCommentsPanelComponent,
};
use crate::page::{CaptureFilter, NetworkCaptureSnapshot, UiSession};

/// Zero-width sentinel characters stripped before comparing comment composer contents to the plan.
pub const SENTINELS: &str = "\u{200b}\u{200c}\u{200d}\u{feff}";

fn strip_sentinels(text: &str) -> String {
    text.chars().filter(|ch| !SENTINELS.contains(*ch)).collect()
}

fn value_as_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Extract and classify the single captured write request and response for `path`.
pub fn captured_write(
    capture: &NetworkCaptureSnapshot,
    path: &str,
    operation: &str,
) -> Result<(BTreeMap<String, String>, Map<String, Value>)> {
    let expected = path.trim_end_matches('/');
    let requests: Vec<BTreeMap<String, String>> = capture
        .requests
        .iter()
        .filter(|req| req.path.trim_end_matches('/') == expected)
        .map(|req| request_fields(&req.url, &req.body))
        .collect();
    let responses: Vec<_> = capture
        .responses
        .iter()
        .filter(|resp| resp.path.trim_end_matches('/') == expected)
        .collect();

    if requests.len() != 1 || responses.len() != 1 {
        let req_paths: Vec<String> = capture
            .requests
            .iter()
            .map(|r| r.path.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let resp_paths: Vec<String> = capture
            .responses
            .iter()
            .map(|r| r.path.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        return Err(DoudouError::new(
            "WRITE_RESULT_UNKNOWN",
            "The page write did not produce one request and one response.",
        )
        .with_details(json!({
            "write_started": true,
            "operation": operation,
            "request_count": requests.len(),
            "response_count": responses.len(),
            "captured_request_paths": req_paths,
            "captured_response_paths": resp_paths,
        })));
    }

    let response = responses[0];
    let parsed = parse_page_write_response(&response.body, response.http_status, operation)?;
    Ok((
        requests.into_iter().next().expect("checked length 1"),
        parsed,
    ))
}

/// Execute a confirmed video comment create or reply write plan and verify its response receipt.
pub async fn comment_write(session: &dyn UiSession, plan: &Value) -> Result<Value> {
    let payload = &plan["payload"];
    let plan_id = value_as_string(plan.get("plan_id"));
    let operation = plan
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("INVALID_PLAN", "Missing operation in plan."))?;
    let aweme_id = payload
        .get("aweme_id")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("INVALID_PLAN", "Missing aweme_id in plan."))?;
    let planned_text = payload
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("INVALID_PLAN", "Missing text in plan."))?;

    let mut operations = VideoCommentsPanelComponent.open(session, aweme_id).await?;
    let page = session.page();
    let item_component = CommentItemComponent::new(page);

    let reply_scope = if operation == "video.comment.reply" {
        let comment_text = payload["comment"]["text"]
            .as_str()
            .ok_or_else(|| DoudouError::new("INVALID_PLAN", "Missing comment.text in plan."))?;
        let author = payload["comment"]
            .get("user")
            .and_then(|u| u.get("nickname"))
            .and_then(Value::as_str);
        let item = item_component.target(comment_text, author, 30).await?;
        operations += item_component.reply(&item).await?;
        page.sleep_ms(500).await?;
        Some(item)
    } else {
        None
    };

    let composer = match reply_scope {
        Some(item) => VideoCommentComposerComponent::scoped(page, item),
        None => VideoCommentComposerComponent::new(page),
    };
    if operation == "video.comment.create" {
        operations += composer.activate().await?;
    }
    let editor = composer.insert_text(planned_text).await?;
    let actual_text = composer.composed_text(&editor).await?;
    if strip_sentinels(&actual_text) != planned_text {
        return Err(DoudouError::new(
            "COMPOSER_PLAN_MISMATCH",
            "The comment editor does not match the confirmed plan.",
        ));
    }

    let publish_path = web_path("comment_publish").unwrap_or("/aweme/v1/web/comment/publish/");
    let publish_trimmed = publish_path.trim_end_matches('/');
    let capture_handle = page
        .start_network_capture(CaptureFilter::contains(["/aweme/v1/web/comment/"]))
        .await?;

    let mut verification_visible = false;
    let mut write_started = false;
    let publish_result: Result<()> = async {
        operations += composer.publish().await?;
        write_started = true;
        operations += 2;
        for _ in 0..250 {
            let snap = capture_handle
                .snapshot()
                .await
                .map_err(DoudouError::mark_write_started)?;
            if snap
                .responses
                .iter()
                .any(|resp| resp.path.trim_end_matches('/') == publish_trimmed)
            {
                break;
            }
            page.sleep_ms(100)
                .await
                .map_err(DoudouError::mark_write_started)?;
        }
        verification_visible = session
            .verification_visible()
            .await
            .map_err(DoudouError::mark_write_started)?;
        Ok(())
    }
    .await;

    let write_started =
        write_started || matches!(&publish_result, Err(err) if err.is_write_started());
    let capture = capture_handle.stop().await.map_err(|err| {
        if write_started {
            err.mark_write_started()
        } else {
            err
        }
    })?;
    publish_result?;

    if verification_visible {
        return Err(DoudouError::new(
            "WRITE_VERIFICATION_REQUIRED",
            "The page showed account verification after the comment write.",
        )
        .with_details(json!({
            "write_started": true,
            "plan_id": plan_id,
        })));
    }

    let (fields, value) = captured_write(&capture, publish_path, operation)?;
    let target_comment_id = if operation == "video.comment.reply" {
        payload
            .get("comment")
            .and_then(|c| c.get("cid"))
            .and_then(Value::as_str)
    } else {
        None
    };
    let verified_fields = validate_comment_request(
        &fields,
        aweme_id,
        Some(planned_text),
        target_comment_id,
        None,
    )?;

    let created = response_comment(&value);
    let created_cid = created
        .and_then(|c| c.get("cid"))
        .map(|v| value_as_string(Some(v)))
        .unwrap_or_default();
    let created_text = created
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if created.is_none() || created_text != planned_text || created_cid.is_empty() {
        return Err(DoudouError::new(
            "WRITE_RESULT_UNKNOWN",
            "The successful page response did not contain the confirmed comment.",
        )
        .with_details(json!({
            "write_started": true,
            "plan_id": plan_id,
            "response_comment_present": created.is_some(),
        })));
    }

    Ok(json!({
        "status": if operation.ends_with("create") { "created" } else { "replied" },
        "plan_id": plan_id,
        "dom_operations": operations,
        "react_fiber_traversal": false,
        "target": verified_fields,
        "created_comment": {
            "cid": created_cid,
            "text": created_text,
        },
        "network": capture.to_network_json(),
        "verification": {
            "passed": true,
            "network_business_status": 0,
        },
    }))
}

/// Execute a confirmed video comment like write plan and verify its exact digg receipt.
pub async fn like_comment(session: &dyn UiSession, plan: &Value) -> Result<Value> {
    let payload = &plan["payload"];
    let plan_id = value_as_string(plan.get("plan_id"));
    let aweme_id = payload
        .get("aweme_id")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("INVALID_PLAN", "Missing aweme_id in plan."))?;
    let comment_text = payload["comment"]["text"]
        .as_str()
        .ok_or_else(|| DoudouError::new("INVALID_PLAN", "Missing comment.text in plan."))?;
    let comment_cid = payload["comment"]["cid"]
        .as_str()
        .ok_or_else(|| DoudouError::new("INVALID_PLAN", "Missing comment.cid in plan."))?;
    let author = payload["comment"]
        .get("user")
        .and_then(|u| u.get("nickname"))
        .and_then(Value::as_str);

    let mut operations = VideoCommentsPanelComponent.open(session, aweme_id).await?;
    let page = session.page();
    let component = CommentItemComponent::new(page);
    let item = component.target(comment_text, author, 30).await?;

    if payload["comment"]
        .get("user_digged")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(json!({
            "status": "already_liked",
            "plan_id": plan_id,
            "dom_operations": operations,
            "verification": {
                "passed": true,
                "already_liked": true,
            },
        }));
    }

    let digg_path = web_path("comment_digg").unwrap_or("/aweme/v1/web/comment/digg/");
    let capture_handle = page
        .start_network_capture(CaptureFilter::exact([digg_path]))
        .await?;

    let digg_count = payload["comment"]
        .get("digg_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    let mut write_started = false;
    let like_result: Result<()> = async {
        operations += component.like(&item, digg_count).await?;
        write_started = true;
        for _ in 0..200 {
            let snap = capture_handle
                .snapshot()
                .await
                .map_err(DoudouError::mark_write_started)?;
            if !snap.responses.is_empty() {
                break;
            }
            page.sleep_ms(100)
                .await
                .map_err(DoudouError::mark_write_started)?;
        }
        Ok(())
    }
    .await;

    let write_started = write_started || matches!(&like_result, Err(err) if err.is_write_started());
    let capture = capture_handle.stop().await.map_err(|err| {
        if write_started {
            err.mark_write_started()
        } else {
            err
        }
    })?;
    like_result?;

    let (fields, _value) = captured_write(&capture, digg_path, "video.comment.like")?;
    let verified_fields =
        validate_comment_request(&fields, aweme_id, None, Some(comment_cid), Some("1"))?;

    Ok(json!({
        "status": "liked",
        "plan_id": plan_id,
        "dom_operations": operations,
        "target": verified_fields,
        "network": capture.to_network_json(),
        "verification": {
            "passed": true,
            "authoritative": true,
            "source": "page_write_response",
            "network_business_status": 0,
        },
    }))
}
