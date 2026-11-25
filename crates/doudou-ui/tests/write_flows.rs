//! Integration tests ported from `test_video_writes.py` plus end-to-end chat/video/auth flows.

use doudou_protocol::{
    bytes_field, encode_fields, parse_page_write_response, request_fields, response_comment,
    string_field, validate_comment_request, varint_field,
};
use doudou_ui::mock::{MockElement, MockUiPage, MockUiSession};
use doudou_ui::{
    ADAPTERS, AUTH_ADAPTER, AuthChallengeComponent, BoundingBox, CHAT_ADAPTER, CaptureFilter,
    CapturedRequest, CapturedResponse, ChatComposerComponent, CommentItemComponent,
    ConversationListComponent, ElementId, ImagePreviewComponent, MessageItemComponent,
    NetworkCaptureSnapshot, PROFILE_ADAPTER, ProfileVideoListComponent, TutorialDialogComponent,
    VIDEO_ADAPTER, VideoCommentComposerComponent, VideoDetailComponent, captured_write,
    comment_write, endpoint_summary, like_comment, like_message, send_message, verify_send_capture,
    visible_comment_text,
};
use serde_json::json;
use tempfile::TempDir;

fn bbox(x: f64, y: f64, width: f64, height: f64) -> BoundingBox {
    BoundingBox::new(x, y, width, height)
}

fn sq(x: f64, y: f64) -> BoundingBox {
    BoundingBox::new(x, y, 20.0, 20.0)
}

fn build_send_request_bytes(
    conv_id: &str,
    conv_type: u64,
    short_id: u64,
    content_json: &str,
    msg_type: u64,
    client_msg_id: &str,
    ref_id: Option<u64>,
) -> Vec<u8> {
    let mut fields = vec![
        string_field(1, conv_id),
        varint_field(2, conv_type),
        varint_field(3, short_id),
        string_field(4, content_json),
        varint_field(6, msg_type),
        string_field(8, client_msg_id),
    ];
    if let Some(rid) = ref_id {
        fields.push(bytes_field(11, encode_fields(&[varint_field(1, rid)])));
    }
    let body = encode_fields(&fields);
    encode_fields(&[
        varint_field(1, 100),
        bytes_field(8, encode_fields(&[bytes_field(100, body)])),
    ])
}

fn build_send_response_bytes(
    server_msg_id: u64,
    business_status: u64,
    client_msg_id: &str,
) -> Vec<u8> {
    let body = encode_fields(&[
        varint_field(1, server_msg_id),
        varint_field(3, business_status),
        string_field(4, client_msg_id),
    ]);
    encode_fields(&[
        varint_field(3, 0),
        string_field(4, "OK"),
        bytes_field(6, encode_fields(&[bytes_field(100, body)])),
    ])
}

// ---------------------------------------------------------------------------
// Ported from test_video_writes.py
// ---------------------------------------------------------------------------

#[test]
fn form_request_and_success_response_match_plan() {
    let fields = request_fields(
        "https://www.douyin.com/aweme/v1/web/comment/publish/?aweme_id=10",
        b"text=%E5%BE%88%E6%A3%92",
    );
    let verified = validate_comment_request(&fields, "10", Some("很棒"), None, None)
        .expect("valid comment publish request");
    assert_eq!(verified["text"], "很棒");

    let value = parse_page_write_response(
        r#"{"status_code":0,"comment":{"cid":"20","text":"很棒"}}"#.as_bytes(),
        200,
        "video.comment.create",
    )
    .expect("valid write response");
    assert_eq!(response_comment(&value).unwrap()["cid"], "20");
}

#[test]
fn comment_like_request_requires_explicit_like_direction() {
    let mut fields = request_fields(
        "https://www.douyin.com/aweme/v1/web/comment/digg/?aweme_id=10&cid=20&digg_type=1",
        b"",
    );
    let verified = validate_comment_request(&fields, "10", None, Some("20"), Some("1"))
        .expect("digg_type=1 accepted");
    assert_eq!(verified["digg_type"], "1");

    fields.insert("digg_type".to_owned(), "0".to_owned());
    let err = validate_comment_request(&fields, "10", None, Some("20"), Some("1"))
        .expect_err("digg_type=0 must fail");
    assert_eq!(err.code, "PAGE_WRITE_REQUEST_DRIFTED");
}

#[test]
fn request_drift_is_unknown_after_click() {
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("aweme_id".to_owned(), "10".to_owned());
    fields.insert("text".to_owned(), "。".to_owned());
    let err = validate_comment_request(&fields, "10", Some("讲得很清楚，辛苦啦。"), None, None)
        .expect_err("text drift fails");
    assert_eq!(err.code, "PAGE_WRITE_REQUEST_DRIFTED");
    assert_eq!(err.details.unwrap()["write_started"], true);
}

#[test]
fn business_failure_and_kick_are_known() {
    let failure = parse_page_write_response(br#"{"status_code":7}"#, 200, "video.comment.like")
        .expect_err("status_code 7 fails");
    assert_eq!(failure.code, "VIDEO_WRITE_FAILED");
    assert_eq!(failure.details.unwrap()["result_known"], true);

    let kick = parse_page_write_response(br#"{"decision":"KICK"}"#, 200, "video.comment.create")
        .expect_err("KICK triggers verification");
    assert_eq!(kick.code, "WRITE_VERIFICATION_REQUIRED");
}

#[test]
fn captured_write_rejects_missing_or_duplicate_exchanges() {
    let empty = NetworkCaptureSnapshot::default();
    let err = captured_write(
        &empty,
        "/aweme/v1/web/comment/publish/",
        "video.comment.create",
    )
    .expect_err("empty capture must fail");
    assert_eq!(err.code, "WRITE_RESULT_UNKNOWN");
    assert_eq!(err.details.unwrap()["request_count"], 0);
}

// ---------------------------------------------------------------------------
// Adapters, endpoint_summary, CaptureFilter, visible_comment_text
// ---------------------------------------------------------------------------

#[tokio::test]
async fn adapters_match_versions_selectors_routes_and_require() {
    assert_eq!(ADAPTERS.len(), 4);
    assert_eq!(CHAT_ADAPTER.version, "2026-08-03.1");
    assert_eq!(
        CHAT_ADAPTER.selector("send_button"),
        Some("svg.e2e-send-msg-btn")
    );
    assert_eq!(
        CHAT_ADAPTER.selector("message_context_menu"),
        Some(".MessageOperatePopWindowwrapper")
    );
    assert!(VIDEO_ADAPTER.matches_route("/video/123"));
    assert!(VIDEO_ADAPTER.matches_route("/note/456"));
    assert!(!VIDEO_ADAPTER.matches_route("/chat"));
    assert!(PROFILE_ADAPTER.matches_route("/user/abc"));
    assert!(AUTH_ADAPTER.matches_route("/"));

    let page = MockUiPage::new();
    let err = CHAT_ADAPTER
        .require(&page, "chat.send", &["composer", "send_button"])
        .await
        .expect_err("missing controls drift");
    assert_eq!(err.code, "PAGE_ADAPTER_DRIFTED");
    assert_eq!(
        err.details.unwrap()["missing_controls"],
        json!(["composer", "send_button"])
    );
}

#[test]
fn endpoint_summary_and_capture_filter_behavior() {
    let summary = endpoint_summary("https://www.douyin.com/aweme/v1/web/comment/digg/?b=2&a=&a=1");
    assert_eq!(summary.origin, "https://www.douyin.com");
    assert_eq!(summary.path, "/aweme/v1/web/comment/digg/");
    assert_eq!(summary.query_keys, vec!["a", "b"]);

    let exact = CaptureFilter::exact(["/aweme/v1/web/comment/digg/"]);
    assert!(exact.matches("https://www.douyin.com/aweme/v1/web/comment/digg?cid=1"));
    assert!(!exact.matches("https://www.douyin.com/aweme/v1/web/comment/publish/"));

    assert_eq!(visible_comment_text("真不错[微笑] [赞]"), "真不错");
}

// ---------------------------------------------------------------------------
// End-to-end video UI flows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn video_comment_create_and_like_end_to_end_flows() {
    let page = MockUiPage::new();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "comment-list",
            "DIV",
            Some(bbox(700.0, 80.0, 520.0, 680.0)),
        ));
        s.add_element(
            MockElement::new("composer-box", "DIV", Some(bbox(710.0, 690.0, 500.0, 60.0)))
                .with_parent("comment-list"),
        );
        s.add_element(
            MockElement::new("editor", "DIV", Some(bbox(720.0, 700.0, 340.0, 30.0)))
                .with_parent("composer-box"),
        );
        s.add_element(
            MockElement::new("emoji-btn", "SPAN", Some(sq(1100.0, 705.0)))
                .with_parent("composer-box"),
        );
        s.add_element(
            MockElement::new("send-btn", "SPAN", Some(sq(1160.0, 705.0)))
                .with_parent("composer-box"),
        );
        s.bind_css(None, r#"[data-e2e="comment-list"]"#, ["comment-list"]);
        s.bind_css(
            None,
            VideoCommentComposerComponent::CONTAINER_SELECTOR,
            ["composer-box"],
        );
        s.bind_css(
            Some("composer-box"),
            VideoCommentComposerComponent::EDITOR_SELECTOR,
            ["editor"],
        );
        s.bind_css(
            Some("composer-box"),
            VideoCommentComposerComponent::CONTROLS_SELECTOR,
            ["emoji-btn", "send-btn"],
        );

        let req = CapturedRequest::new(
            "POST",
            "https://www.douyin.com/aweme/v1/web/comment/publish/?aweme_id=777",
            b"text=%E5%86%99%E5%BE%97%E5%A5%BD".to_vec(),
        );
        let resp = CapturedResponse::new(
            "https://www.douyin.com/aweme/v1/web/comment/publish/?aweme_id=777",
            200,
            r#"{"status_code":0,"comment":{"cid":"c-99","text":"写得好"}}"#
                .as_bytes()
                .to_vec(),
        );
        s.staged_network.push((Some(req), Some(resp)));
    });

    let session = MockUiSession::new(page);
    let plan = json!({
        "plan_id": "plan-video-1",
        "operation": "video.comment.create",
        "payload": {
            "aweme_id": "777",
            "text": "写得好",
        },
    });
    let result = comment_write(&session, &plan)
        .await
        .expect("comment_write succeeds");
    assert_eq!(result["status"], "created");
    assert_eq!(result["created_comment"]["cid"], "c-99");
    assert_eq!(result["verification"]["passed"], true);

    // Now test like_comment on a comment item.
    let like_page = MockUiPage::new();
    like_page.with_state(|s| {
        s.add_element(MockElement::new(
            "comment-list",
            "DIV",
            Some(bbox(700.0, 80.0, 520.0, 680.0)),
        ));
        s.add_element(
            MockElement::new("c-item", "DIV", Some(bbox(710.0, 120.0, 500.0, 120.0)))
                .with_parent("comment-list"),
        );
        s.add_element(
            MockElement::new("c-text", "SPAN", Some(bbox(720.0, 130.0, 300.0, 24.0)))
                .with_parent("c-item")
                .with_text("目标评论"),
        );
        s.add_element(
            MockElement::new("c-author", "SPAN", Some(bbox(720.0, 120.0, 120.0, 18.0)))
                .with_parent("c-item")
                .with_text("作者A"),
        );
        s.add_element(
            MockElement::new("c-stats", "DIV", Some(bbox(715.0, 180.0, 200.0, 30.0)))
                .with_parent("c-item"),
        );
        s.add_element(
            MockElement::new("c-like-btn", "BUTTON", Some(bbox(720.0, 182.0, 60.0, 24.0)))
                .with_parent("c-stats")
                .with_attr("aria-pressed", "false"),
        );
        s.add_element(
            MockElement::new("c-like-count", "P", Some(bbox(740.0, 184.0, 30.0, 20.0)))
                .with_parent("c-like-btn")
                .with_text("12"),
        );
        s.add_element(
            MockElement::new("c-like-svg", "SVG", Some(bbox(722.0, 184.0, 16.0, 16.0)))
                .with_parent("c-like-btn"),
        );

        s.bind_css(None, r#"[data-e2e="comment-list"]"#, ["comment-list"]);
        s.bind_xpath(Some("c-text"), CommentItemComponent::ITEM_XPATH, ["c-item"]);
        s.bind_css(
            Some("c-item"),
            CommentItemComponent::STATS_SELECTOR,
            ["c-stats"],
        );
        s.bind_xpath(
            Some("c-like-count"),
            CommentItemComponent::LIKE_CONTROL_XPATH,
            ["c-like-btn"],
        );
        s.bind_css(
            Some("c-like-btn"),
            CommentItemComponent::LIKE_EVIDENCE_SELECTOR,
            ["c-like-svg"],
        );
    });
    like_page.set_click_hook(|s, elem, _btn| {
        if elem.as_str() == "c-like-btn" {
            let req = CapturedRequest::new(
                "POST",
                "https://www.douyin.com/aweme/v1/web/comment/digg/?aweme_id=777&cid=c-1&digg_type=1",
                Vec::new(),
            );
            let resp = CapturedResponse::new(
                "https://www.douyin.com/aweme/v1/web/comment/digg/?aweme_id=777&cid=c-1&digg_type=1",
                200,
                br#"{"status_code":0}"#.to_vec(),
            );
            s.emit_network(Some(&req), Some(&resp));
        }
        Ok(())
    });

    let like_session = MockUiSession::new(like_page);
    let like_plan = json!({
        "plan_id": "plan-like-1",
        "operation": "video.comment.like",
        "payload": {
            "aweme_id": "777",
            "comment": {
                "cid": "c-1",
                "text": "目标评论[赞]",
                "digg_count": 12,
                "user_digged": false,
                "user": { "nickname": "作者A" },
            },
        },
    });
    let liked = like_comment(&like_session, &like_plan)
        .await
        .expect("like_comment succeeds");
    assert_eq!(liked["status"], "liked");
    assert_eq!(liked["target"]["digg_type"], "1");
    assert_eq!(liked["verification"]["authoritative"], true);
}

// ---------------------------------------------------------------------------
// End-to-end chat UI write flows (send text, send image, like message)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn chat_send_message_and_image_and_like_flows() {
    let page = MockUiPage::new();
    let conv_list_sel = CHAT_ADAPTER.selector("conversation_list").unwrap();
    let session_sel = CHAT_ADAPTER.selector("session_candidates").unwrap();

    page.with_state(|s| {
        s.add_element(MockElement::new(
            "conv-list",
            "DIV",
            Some(bbox(0.0, 100.0, 320.0, 400.0)),
        ));
        s.add_element(
            MockElement::new("row-1", "DIV", Some(bbox(20.0, 120.0, 280.0, 60.0)))
                .with_parent("conv-list")
                .with_class(ConversationListComponent::SELECTED_CLASS)
                .with_text("测试好友"),
        );
        s.add_element(MockElement::new(
            "input-row",
            "DIV",
            Some(bbox(320.0, 700.0, 900.0, 80.0)),
        ));
        s.add_element(
            MockElement::new("editor", "DIV", Some(bbox(340.0, 710.0, 600.0, 40.0)))
                .with_parent("input-row"),
        );
        s.add_element(
            MockElement::new("emoji-svg", "SVG", Some(sq(1000.0, 720.0))).with_parent("input-row"),
        );
        s.add_element(
            MockElement::new("send-svg", "SVG", Some(sq(1160.0, 720.0))).with_parent("input-row"),
        );

        s.bind_css(None, conv_list_sel, ["conv-list"]);
        s.bind_css(Some("conv-list"), session_sel, ["row-1"]);
        s.bind_css(None, ChatComposerComponent::COMPOSER_SELECTOR, ["editor"]);
        s.bind_xpath(
            Some("editor"),
            ChatComposerComponent::CONTAINER_XPATH,
            ["input-row"],
        );
        s.bind_css(
            Some("input-row"),
            ChatComposerComponent::SEND_SELECTOR,
            ["send-svg"],
        );
        s.bind_css(Some("input-row"), "svg", ["emoji-svg", "send-svg"]);

        let req_bytes = build_send_request_bytes(
            "0:1:10:20",
            1,
            999,
            r#"{"text":"你好世界"}"#,
            7,
            "client-1",
            None,
        );
        let resp_bytes = build_send_response_bytes(888_888, 0, "client-1");
        s.staged_network.push((
            Some(CapturedRequest::new(
                "POST",
                "https://imapi.douyin.com/v1/message/send",
                req_bytes,
            )),
            Some(CapturedResponse::new(
                "https://imapi.douyin.com/v1/message/send",
                200,
                resp_bytes,
            )),
        ));
    });
    page.set_eval_hook(|elem, script, _arg| {
        if script.contains("document.activeElement") {
            return Ok(json!(elem.as_str() == "editor"));
        }
        Ok(json!({ "resolved": true, "moved": false }))
    });

    let session = MockUiSession::new(page);
    let plan = json!({
        "plan_id": "plan-chat-1",
        "payload": {
            "mode": "text",
            "text": "你好世界",
            "conversation": {
                "display_name": "测试好友",
                "conversation_id": "0:1:10:20",
                "conversation_short_id": "999",
                "conversation_type": 1,
            },
        },
    });
    let receipt = send_message(&session, &plan)
        .await
        .expect("send_message succeeds");
    assert_eq!(receipt["status"], "sent");
    assert_eq!(receipt["verification"]["passed"], true);
    assert_eq!(receipt["verification"]["server_message_id"], "888888");

    // Verify image mode verification helper directly.
    let img_plan = json!({
        "plan_id": "plan-img-1",
        "payload": {
            "mode": "image",
            "conversation": {
                "display_name": "测试好友",
                "conversation_id": "0:1:10:20",
                "conversation_short_id": "999",
                "conversation_type": 1,
            },
        },
    });
    let img_req = build_send_request_bytes(
        "0:1:10:20",
        1,
        999,
        r#"{"aweType":2702,"resource_url":{"oid":"oid-1","skey":"skey-1"}}"#,
        27,
        "client-img-1",
        None,
    );
    let img_resp = build_send_response_bytes(999_001, 0, "client-img-1");
    let img_snap = NetworkCaptureSnapshot {
        requests: vec![CapturedRequest::new(
            "POST",
            "https://imapi.douyin.com/v1/message/send",
            img_req,
        )],
        responses: vec![CapturedResponse::new(
            "https://imapi.douyin.com/v1/message/send",
            200,
            img_resp,
        )],
    };
    let img_verification = verify_send_capture(&img_snap, &img_plan);
    assert_eq!(img_verification["passed"], true);
    assert_eq!(img_verification["image_shape_match"], true);

    // Verify ImagePreviewComponent choose + send flow.
    let dir = TempDir::new().unwrap();
    let img_file = dir.path().join("sample.png");
    std::fs::write(&img_file, b"\x89PNG\r\n\x1a\n").unwrap();

    let img_page = MockUiPage::new();
    img_page.with_state(|s| {
        s.add_element(MockElement::new(
            "msg-input",
            "DIV",
            Some(bbox(320.0, 680.0, 900.0, 100.0)),
        ));
        s.add_element(
            MockElement::new("upload-btn", "BUTTON", Some(sq(340.0, 690.0)))
                .with_parent("msg-input"),
        );
        s.add_element(MockElement::new(
            "preview-dialog",
            "DIV",
            Some(bbox(400.0, 200.0, 480.0, 360.0)),
        ));
        s.add_element(
            MockElement::new("preview-img", "IMG", Some(bbox(440.0, 240.0, 400.0, 200.0)))
                .with_parent("preview-dialog"),
        );
        s.add_element(
            MockElement::new(
                "preview-name",
                "SPAN",
                Some(bbox(440.0, 450.0, 120.0, 20.0)),
            )
            .with_parent("preview-dialog")
            .with_text("sample.png"),
        );
        s.add_element(
            MockElement::new(
                "preview-send",
                "BUTTON",
                Some(bbox(760.0, 500.0, 80.0, 32.0)),
            )
            .with_parent("preview-dialog"),
        );
        s.bind_css(None, r#"[data-e2e="msg-input"]"#, ["msg-input"]);
        s.bind_css(
            Some("msg-input"),
            ImagePreviewComponent::UPLOAD_SELECTOR,
            ["upload-btn"],
        );
        s.bind_css(
            None,
            CHAT_ADAPTER.selector("image_preview_dialog").unwrap(),
            ["preview-dialog"],
        );
        s.bind_css(Some("preview-dialog"), "img", ["preview-img"]);
        s.bind_css(
            Some("preview-dialog"),
            CHAT_ADAPTER.selector("image_preview_send").unwrap(),
            ["preview-send"],
        );
    });
    let mut preview = ImagePreviewComponent::new(&img_page);
    assert_eq!(preview.choose(&img_file).await.unwrap(), 2);
    assert_eq!(preview.send().await.unwrap(), 1);

    // Verify like_message (already_liked short-circuit + context menu flow).
    let already = like_message(
        &session,
        &json!({
            "plan_id": "plan-mlike-0",
            "payload": { "already_liked": true },
        }),
    )
    .await
    .unwrap();
    assert_eq!(already["status"], "already_liked");

    let mlike_page = MockUiPage::new();
    mlike_page.with_state(|s| {
        s.add_element(MockElement::new(
            "conv-list",
            "DIV",
            Some(bbox(0.0, 100.0, 320.0, 400.0)),
        ));
        s.add_element(
            MockElement::new("row-1", "DIV", Some(bbox(20.0, 120.0, 280.0, 60.0)))
                .with_parent("conv-list")
                .with_class(ConversationListComponent::SELECTED_CLASS)
                .with_text("测试好友"),
        );
        s.add_element(
            MockElement::new("msg-1", "DIV", Some(bbox(420.0, 300.0, 200.0, 40.0)))
                .with_text("要点赞的消息"),
        );
        s.add_element(
            MockElement::new("ctx-menu", "DIV", Some(bbox(430.0, 350.0, 120.0, 100.0)))
                .with_class(MessageItemComponent::MENU_READY_CLASS),
        );
        s.add_element(
            MockElement::new("ctx-like", "DIV", Some(bbox(435.0, 360.0, 100.0, 28.0)))
                .with_parent("ctx-menu")
                .with_text("点赞"),
        );
        s.bind_css(None, conv_list_sel, ["conv-list"]);
        s.bind_css(Some("conv-list"), session_sel, ["row-1"]);
        s.bind_css(None, MessageItemComponent::ITEM_SELECTOR, ["msg-1"]);
        s.bind_css(
            None,
            CHAT_ADAPTER.selector("message_context_menu").unwrap(),
            ["ctx-menu"],
        );
        s.bind_css(
            Some("ctx-menu"),
            CHAT_ADAPTER.selector("message_context_action").unwrap(),
            ["ctx-like"],
        );
    });
    mlike_page.set_eval_hook(|elem, script, _arg| {
        if script.contains("media_labels") {
            return Ok(json!({ "text": "要点赞的消息", "media_labels": [] }));
        }
        if elem.as_str() == "conv-list" {
            return Ok(json!({ "resolved": true, "moved": false }));
        }
        Ok(serde_json::Value::Null)
    });
    mlike_page.set_point_click_hook(|s, _x, _y, _btn| {
        let req = CapturedRequest::new(
            "POST",
            "https://imapi.douyin.com/v1/message/modify_property",
            vec![1, 2, 3],
        );
        let resp = CapturedResponse::new(
            "https://imapi.douyin.com/v1/message/modify_property",
            200,
            vec![0],
        );
        s.emit_network(Some(&req), Some(&resp));
        Ok(())
    });
    let mlike_session = MockUiSession::new(mlike_page);
    let liked_msg = like_message(
        &mlike_session,
        &json!({
            "plan_id": "plan-mlike-1",
            "payload": {
                "already_liked": false,
                "conversation": { "display_name": "测试好友" },
                "message": { "text": "要点赞的消息" },
            },
        }),
    )
    .await
    .expect("like_message succeeds");
    assert_eq!(liked_msg["status"], "liked");
    assert_eq!(liked_msg["verification"]["transport_observed"], true);
}

// ---------------------------------------------------------------------------
// AuthChallengeComponent & auxiliary video/profile components
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_challenge_detects_quick_login_sms_and_requests_code() {
    let page = MockUiPage::new();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "sms-dialog",
            "DIV",
            Some(bbox(300.0, 200.0, 420.0, 300.0)),
        ));
        s.add_element(
            MockElement::new("sms-input", "INPUT", Some(bbox(340.0, 280.0, 200.0, 36.0)))
                .with_parent("sms-dialog"),
        );
        s.add_element(
            MockElement::new("quick-title", "H2", Some(bbox(340.0, 220.0, 160.0, 24.0)))
                .with_parent("sms-dialog")
                .with_text("快捷登录"),
        );
        s.add_element(
            MockElement::new(
                "get-code-btn",
                "BUTTON",
                Some(bbox(560.0, 280.0, 120.0, 36.0)),
            )
            .with_parent("sms-dialog")
            .with_text("获取验证码"),
        );
        s.bind_css(
            None,
            AUTH_ADAPTER.selector("sms_code").unwrap(),
            ["sms-input"],
        );
        s.bind_xpath(
            Some("sms-input"),
            AuthChallengeComponent::CONTAINER_XPATH,
            ["sms-dialog"],
        );
    });

    let auth = AuthChallengeComponent::new(&page);
    assert_eq!(
        auth.detect().await.unwrap().as_deref(),
        Some("quick_login_sms")
    );
    assert!(auth.request_sms_code().await.unwrap());
    assert_eq!(
        page.with_state(|s| s.elements[&ElementId::from("get-code-btn")].click_calls),
        1
    );

    let probes = auth.probe(true).await.unwrap();
    assert_eq!(probes.len(), 5);
    assert_eq!(probes[0]["status"], "ok");
    assert_eq!(probes[1]["status"], "not_exercised");
}

#[tokio::test]
async fn tutorial_profile_and_video_detail_components_work() {
    let page = MockUiPage::new();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "tut-dialog",
            "DIV",
            Some(bbox(400.0, 200.0, 300.0, 200.0)),
        ));
        s.add_element(
            MockElement::new("tut-ok", "BUTTON", Some(bbox(480.0, 340.0, 100.0, 32.0)))
                .with_parent("tut-dialog")
                .with_text("我知道了"),
        );
        s.bind_xpath(
            Some("tut-ok"),
            TutorialDialogComponent::DIALOG_XPATH,
            ["tut-dialog"],
        );
        s.add_element(MockElement::new(
            "scroll-root",
            "DIV",
            Some(bbox(0.0, 0.0, 1280.0, 800.0)),
        ));
        s.bind_css(
            None,
            PROFILE_ADAPTER.selector("scroll_container").unwrap(),
            ["scroll-root"],
        );
        s.add_element(
            MockElement::new("render-data", "SCRIPT", None).with_text("%7B%22ok%22%3Atrue%7D"),
        );
        s.bind_css(
            None,
            VIDEO_ADAPTER.selector("render_data").unwrap(),
            ["render-data"],
        );
    });
    page.set_eval_hook(|_e, _s, _a| Ok(json!(true)));

    assert_eq!(TutorialDialogComponent.dismiss(&page).await.unwrap(), 1);
    assert!(
        ProfileVideoListComponent::new(&page)
            .scroll_to_end()
            .await
            .unwrap()
    );
    assert_eq!(
        VideoDetailComponent::new(&page)
            .render_data()
            .await
            .unwrap()
            .as_deref(),
        Some("%7B%22ok%22%3Atrue%7D")
    );
}

/// Regression tests for Finding 3: post-submit UI click, polling, and capture-stop errors
/// across all four UI write flows (`send_message` text/image, `like_message`, `comment_write`,
/// `like_comment`) must attach `"write_started": true` in `details`.
#[tokio::test]
async fn post_submit_polling_and_capture_failures_set_write_started_true() {
    let conv_list_sel = CHAT_ADAPTER.selector("conversation_list").unwrap();
    let session_sel = CHAT_ADAPTER.selector("session_candidates").unwrap();

    // 1. send_message (text): capture_handle.snapshot() fails after Enter submit.
    {
        let page = MockUiPage::new();
        page.with_state(|s| {
            s.add_element(MockElement::new(
                "conv-list",
                "DIV",
                Some(bbox(0.0, 100.0, 320.0, 400.0)),
            ));
            s.add_element(
                MockElement::new("row-1", "DIV", Some(bbox(20.0, 120.0, 280.0, 60.0)))
                    .with_parent("conv-list")
                    .with_class(ConversationListComponent::SELECTED_CLASS)
                    .with_text("测试好友"),
            );
            s.add_element(MockElement::new(
                "input-row",
                "DIV",
                Some(bbox(320.0, 700.0, 900.0, 80.0)),
            ));
            s.add_element(
                MockElement::new("editor", "DIV", Some(bbox(340.0, 710.0, 600.0, 40.0)))
                    .with_parent("input-row"),
            );
            s.add_element(
                MockElement::new("emoji-svg", "SVG", Some(sq(1000.0, 720.0)))
                    .with_parent("input-row"),
            );
            s.add_element(
                MockElement::new("send-svg", "SVG", Some(sq(1160.0, 720.0)))
                    .with_parent("input-row"),
            );
            s.bind_css(None, conv_list_sel, ["conv-list"]);
            s.bind_css(Some("conv-list"), session_sel, ["row-1"]);
            s.bind_css(None, ChatComposerComponent::COMPOSER_SELECTOR, ["editor"]);
            s.bind_xpath(
                Some("editor"),
                ChatComposerComponent::CONTAINER_XPATH,
                ["input-row"],
            );
            s.bind_css(
                Some("input-row"),
                ChatComposerComponent::SEND_SELECTOR,
                ["send-svg"],
            );
            s.bind_css(Some("input-row"), "svg", ["emoji-svg", "send-svg"]);
            s.capture_snapshot_error = Some(doudou_core::DoudouError::new(
                "CDP_ERROR",
                "snapshot failed after submit",
            ));
        });
        page.set_eval_hook(|elem, script, _arg| {
            if script.contains("document.activeElement") {
                return Ok(json!(elem.as_str() == "editor"));
            }
            Ok(json!({ "resolved": true, "moved": false }))
        });
        let session = MockUiSession::new(page);
        let plan = json!({
            "plan_id": "plan-chat-poll-err",
            "payload": {
                "mode": "text",
                "text": "你好",
                "conversation": { "display_name": "测试好友" },
            },
        });
        let err = send_message(&session, &plan)
            .await
            .expect_err("snapshot error must propagate");
        assert_eq!(err.code, "CDP_ERROR");
        assert_eq!(
            err.details.as_ref().expect("details")["write_started"],
            true
        );
    }

    // 2. send_message (image): click_element error on image_preview.send() sets write_started=true.
    {
        let dir = TempDir::new().unwrap();
        let img_file = dir.path().join("sample.png");
        std::fs::write(&img_file, b"\x89PNG\r\n\x1a\n").unwrap();

        let page = MockUiPage::new();
        page.with_state(|s| {
            s.add_element(MockElement::new(
                "conv-list",
                "DIV",
                Some(bbox(0.0, 100.0, 320.0, 400.0)),
            ));
            s.add_element(
                MockElement::new("row-1", "DIV", Some(bbox(20.0, 120.0, 280.0, 60.0)))
                    .with_parent("conv-list")
                    .with_class(ConversationListComponent::SELECTED_CLASS)
                    .with_text("测试好友"),
            );
            s.add_element(MockElement::new(
                "msg-input",
                "DIV",
                Some(bbox(320.0, 680.0, 900.0, 100.0)),
            ));
            s.add_element(
                MockElement::new("upload-btn", "BUTTON", Some(sq(340.0, 690.0)))
                    .with_parent("msg-input"),
            );
            s.add_element(MockElement::new(
                "preview-dialog",
                "DIV",
                Some(bbox(400.0, 200.0, 480.0, 360.0)),
            ));
            s.add_element(
                MockElement::new("preview-img", "IMG", Some(bbox(440.0, 240.0, 400.0, 200.0)))
                    .with_parent("preview-dialog"),
            );
            s.add_element(
                MockElement::new(
                    "preview-name",
                    "SPAN",
                    Some(bbox(440.0, 450.0, 120.0, 20.0)),
                )
                .with_parent("preview-dialog")
                .with_text("sample.png"),
            );
            let mut send_btn = MockElement::new(
                "preview-send",
                "BUTTON",
                Some(bbox(760.0, 500.0, 80.0, 32.0)),
            )
            .with_parent("preview-dialog");
            send_btn
                .click_errors
                .push_back(doudou_core::DoudouError::new(
                    "CDP_ERROR",
                    "click transport drop",
                ));
            s.add_element(send_btn);

            s.bind_css(None, conv_list_sel, ["conv-list"]);
            s.bind_css(Some("conv-list"), session_sel, ["row-1"]);
            s.bind_css(None, r#"[data-e2e="msg-input"]"#, ["msg-input"]);
            s.bind_css(
                Some("msg-input"),
                ImagePreviewComponent::UPLOAD_SELECTOR,
                ["upload-btn"],
            );
            s.bind_css(
                None,
                CHAT_ADAPTER.selector("image_preview_dialog").unwrap(),
                ["preview-dialog"],
            );
            s.bind_css(Some("preview-dialog"), "img", ["preview-img"]);
            s.bind_css(
                Some("preview-dialog"),
                CHAT_ADAPTER.selector("image_preview_send").unwrap(),
                ["preview-send"],
            );
        });
        page.set_eval_hook(|_elem, _script, _arg| Ok(json!({ "resolved": true, "moved": false })));
        let session = MockUiSession::new(page);
        let plan = json!({
            "plan_id": "plan-img-click-err",
            "payload": {
                "mode": "image",
                "image_path": img_file.display().to_string(),
                "conversation": { "display_name": "测试好友" },
            },
        });
        let err = send_message(&session, &plan)
            .await
            .expect_err("image send click error must propagate");
        assert_eq!(err.code, "CDP_ERROR");
        assert_eq!(
            err.details.as_ref().expect("details")["write_started"],
            true
        );
    }

    // 3. like_message: capture_handle.stop() fails after clicking 点赞.
    {
        let page = MockUiPage::new();
        page.with_state(|s| {
            s.add_element(MockElement::new(
                "conv-list",
                "DIV",
                Some(bbox(0.0, 100.0, 320.0, 400.0)),
            ));
            s.add_element(
                MockElement::new("row-1", "DIV", Some(bbox(20.0, 120.0, 280.0, 60.0)))
                    .with_parent("conv-list")
                    .with_class(ConversationListComponent::SELECTED_CLASS)
                    .with_text("测试好友"),
            );
            s.add_element(
                MockElement::new("msg-1", "DIV", Some(bbox(420.0, 300.0, 200.0, 40.0)))
                    .with_text("要点赞的消息"),
            );
            s.add_element(
                MockElement::new("ctx-menu", "DIV", Some(bbox(430.0, 350.0, 120.0, 100.0)))
                    .with_class(MessageItemComponent::MENU_READY_CLASS),
            );
            s.add_element(
                MockElement::new("ctx-like", "DIV", Some(bbox(435.0, 360.0, 100.0, 28.0)))
                    .with_parent("ctx-menu")
                    .with_text("点赞"),
            );
            s.bind_css(None, conv_list_sel, ["conv-list"]);
            s.bind_css(Some("conv-list"), session_sel, ["row-1"]);
            s.bind_css(None, MessageItemComponent::ITEM_SELECTOR, ["msg-1"]);
            s.bind_css(
                None,
                CHAT_ADAPTER.selector("message_context_menu").unwrap(),
                ["ctx-menu"],
            );
            s.bind_css(
                Some("ctx-menu"),
                CHAT_ADAPTER.selector("message_context_action").unwrap(),
                ["ctx-like"],
            );
            s.capture_stop_error = Some(doudou_core::DoudouError::new(
                "CDP_ERROR",
                "stop failed after like",
            ));
        });
        page.set_eval_hook(|elem, script, _arg| {
            if script.contains("media_labels") {
                return Ok(json!({ "text": "要点赞的消息", "media_labels": [] }));
            }
            if elem.as_str() == "conv-list" {
                return Ok(json!({ "resolved": true, "moved": false }));
            }
            Ok(serde_json::Value::Null)
        });
        let session = MockUiSession::new(page);
        let err = like_message(
            &session,
            &json!({
                "plan_id": "plan-mlike-stop-err",
                "payload": {
                    "already_liked": false,
                    "conversation": { "display_name": "测试好友" },
                    "message": { "text": "要点赞的消息" },
                },
            }),
        )
        .await
        .expect_err("stop error after like click must propagate");
        assert_eq!(err.code, "CDP_ERROR");
        assert_eq!(
            err.details.as_ref().expect("details")["write_started"],
            true
        );
    }

    // 4. comment_write: capture_handle.snapshot() fails after Enter publish.
    {
        let page = MockUiPage::new();
        page.with_state(|s| {
            s.add_element(MockElement::new(
                "comment-list",
                "DIV",
                Some(bbox(700.0, 80.0, 520.0, 680.0)),
            ));
            s.add_element(
                MockElement::new("composer-box", "DIV", Some(bbox(710.0, 690.0, 500.0, 60.0)))
                    .with_parent("comment-list"),
            );
            s.add_element(
                MockElement::new("editor", "DIV", Some(bbox(720.0, 700.0, 340.0, 30.0)))
                    .with_parent("composer-box"),
            );
            s.add_element(
                MockElement::new("emoji-btn", "SPAN", Some(sq(1100.0, 705.0)))
                    .with_parent("composer-box"),
            );
            s.add_element(
                MockElement::new("send-btn", "SPAN", Some(sq(1160.0, 705.0)))
                    .with_parent("composer-box"),
            );
            s.bind_css(None, r#"[data-e2e="comment-list"]"#, ["comment-list"]);
            s.bind_css(
                None,
                VideoCommentComposerComponent::CONTAINER_SELECTOR,
                ["composer-box"],
            );
            s.bind_css(
                Some("composer-box"),
                VideoCommentComposerComponent::EDITOR_SELECTOR,
                ["editor"],
            );
            s.bind_css(
                Some("composer-box"),
                VideoCommentComposerComponent::CONTROLS_SELECTOR,
                ["emoji-btn", "send-btn"],
            );
            s.capture_snapshot_error = Some(doudou_core::DoudouError::new(
                "CDP_ERROR",
                "comment snapshot failed",
            ));
        });
        let session = MockUiSession::new(page);
        let plan = json!({
            "plan_id": "plan-video-poll-err",
            "operation": "video.comment.create",
            "payload": { "aweme_id": "777", "text": "写得好" },
        });
        let err = comment_write(&session, &plan)
            .await
            .expect_err("comment snapshot error must propagate");
        assert_eq!(err.code, "CDP_ERROR");
        assert_eq!(
            err.details.as_ref().expect("details")["write_started"],
            true
        );
    }

    // 5. like_comment: capture_handle.snapshot() fails after clicking like button.
    {
        let page = MockUiPage::new();
        page.with_state(|s| {
            s.add_element(MockElement::new(
                "comment-list",
                "DIV",
                Some(bbox(700.0, 80.0, 520.0, 680.0)),
            ));
            s.add_element(
                MockElement::new("c-item", "DIV", Some(bbox(710.0, 120.0, 500.0, 120.0)))
                    .with_parent("comment-list"),
            );
            s.add_element(
                MockElement::new("c-text", "SPAN", Some(bbox(720.0, 130.0, 300.0, 24.0)))
                    .with_parent("c-item")
                    .with_text("目标评论"),
            );
            s.add_element(
                MockElement::new("c-author", "SPAN", Some(bbox(720.0, 120.0, 120.0, 18.0)))
                    .with_parent("c-item")
                    .with_text("作者A"),
            );
            s.add_element(
                MockElement::new("c-stats", "DIV", Some(bbox(715.0, 180.0, 200.0, 30.0)))
                    .with_parent("c-item"),
            );
            s.add_element(
                MockElement::new("c-like-btn", "BUTTON", Some(bbox(720.0, 182.0, 60.0, 24.0)))
                    .with_parent("c-stats")
                    .with_attr("aria-pressed", "false"),
            );
            s.add_element(
                MockElement::new("c-like-count", "P", Some(bbox(740.0, 184.0, 30.0, 20.0)))
                    .with_parent("c-like-btn")
                    .with_text("12"),
            );
            s.add_element(
                MockElement::new("c-like-svg", "SVG", Some(bbox(722.0, 184.0, 16.0, 16.0)))
                    .with_parent("c-like-btn"),
            );
            s.bind_css(None, r#"[data-e2e="comment-list"]"#, ["comment-list"]);
            s.bind_xpath(Some("c-text"), CommentItemComponent::ITEM_XPATH, ["c-item"]);
            s.bind_css(
                Some("c-item"),
                CommentItemComponent::STATS_SELECTOR,
                ["c-stats"],
            );
            s.bind_xpath(
                Some("c-like-count"),
                CommentItemComponent::LIKE_CONTROL_XPATH,
                ["c-like-btn"],
            );
            s.bind_css(
                Some("c-like-btn"),
                CommentItemComponent::LIKE_EVIDENCE_SELECTOR,
                ["c-like-svg"],
            );
            s.capture_snapshot_error = Some(doudou_core::DoudouError::new(
                "CDP_ERROR",
                "like snapshot failed",
            ));
        });
        let session = MockUiSession::new(page);
        let plan = json!({
            "plan_id": "plan-like-poll-err",
            "operation": "video.comment.like",
            "payload": {
                "aweme_id": "777",
                "comment": {
                    "cid": "c-1",
                    "text": "目标评论[赞]",
                    "digg_count": 12,
                    "user_digged": false,
                    "user": { "nickname": "作者A" },
                },
            },
        });
        let err = like_comment(&session, &plan)
            .await
            .expect_err("like snapshot error must propagate");
        assert_eq!(err.code, "CDP_ERROR");
        assert_eq!(
            err.details.as_ref().expect("details")["write_started"],
            true
        );
    }
}
