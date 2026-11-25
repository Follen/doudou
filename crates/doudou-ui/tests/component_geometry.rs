//! Integration tests ported from `test_component_geometry.py` plus PR #8 conversation-list fixes.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use doudou_core::DoudouError;
use doudou_ui::mock::{MockElement, MockUiPage};
use doudou_ui::{
    AnchorRelation, BoundingBox, CHAT_ADAPTER, ChatComposerComponent, ClickOptions,
    CommentItemComponent, ComponentContext, ConversationListComponent, ElementId, MessageEvidence,
    PeerPosition, PointHit, SelectorQuery, TargetOptions, VideoCommentComposerComponent,
    VideoCommentsPanelComponent, click_verified_target, component_probe, message_semantics_match,
    require_target,
};
use serde_json::{Value, json};

fn bbox(x: f64, y: f64, width: f64, height: f64) -> BoundingBox {
    BoundingBox::new(x, y, width, height)
}

fn sq(x: f64, y: f64) -> BoundingBox {
    BoundingBox::new(x, y, 20.0, 20.0)
}

fn fixture_ctx() -> ComponentContext<'static> {
    ComponentContext::new(
        "fixture-component",
        "test.v1",
        "fixture.write",
        "fixture semantic -> local -> geometry",
    )
}

async fn assert_drift(
    page: &MockUiPage,
    candidates: &[ElementId],
    expected_reason: &str,
    options: &TargetOptions<'_>,
) -> DoudouError {
    let err = require_target(page, candidates, &fixture_ctx(), options)
        .await
        .expect_err("expected PAGE_ADAPTER_DRIFTED");
    assert_eq!(err.code, "PAGE_ADAPTER_DRIFTED");
    let reason = err
        .details
        .as_ref()
        .and_then(|d| d.get("geometry_reason"))
        .and_then(Value::as_str);
    assert_eq!(reason, Some(expected_reason), "details: {:?}", err.details);
    err
}

#[tokio::test]
async fn comment_scroll_prefers_unique_main_content() {
    let page = MockUiPage::new();
    let root =
        page.with_state(|s| s.add_element(MockElement::new("root", "DIV", Some(sq(0.0, 0.0)))));
    page.set_eval_hook(|_elem, _script, _arg| {
        Ok(json!({
            "resolved": true,
            "source": "comment-mainContent",
            "moved": true,
        }))
    });

    let result = CommentItemComponent::new(&page)
        .scroll_toward_end(&root)
        .await
        .expect("scroll succeeds");
    assert_eq!(result["source"], "comment-mainContent");
    let script = page.with_state(|s| s.recorded_scripts.last().cloned().unwrap_or_default());
    assert!(script.contains(".comment-mainContent"));
    assert!(script.contains("node.matches('.comment-mainContent')"));
    assert!(script.contains("preferred.length !== 1"));
}

#[tokio::test]
async fn reply_template_retries_once_after_click_failure() {
    let page = MockUiPage::new();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "panel",
            "DIV",
            Some(bbox(0.0, 0.0, 400.0, 400.0)),
        ));
        let mut trigger = MockElement::new("trigger", "BUTTON", Some(sq(10.0, 20.0)))
            .with_parent("panel")
            .with_text("展开 2 条回复");
        trigger
            .click_errors
            .push_back(DoudouError::new("TIMEOUT", "first"));
        s.add_element(trigger);
        s.bind_css(None, r#"[data-e2e="comment-list"]"#, ["panel"]);
        s.bind_css(
            Some("panel"),
            VideoCommentsPanelComponent::REPLY_EXPAND_SELECTOR,
            ["trigger"],
        );
    });

    let result = VideoCommentsPanelComponent
        .trigger_reply_template(&page, 2)
        .await
        .expect("retry succeeds on second attempt");

    assert_eq!(result, 1);
    assert_eq!(
        page.with_state(|s| s.elements[&ElementId::from("trigger")].click_calls),
        1
    );
    assert_eq!(page.with_state(|s| s.waits_ms.clone()), vec![150]);
}

#[tokio::test]
async fn reply_template_stops_at_retry_limit() {
    let page = MockUiPage::new();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "panel",
            "DIV",
            Some(bbox(0.0, 0.0, 400.0, 400.0)),
        ));
        let mut trigger = MockElement::new("trigger", "BUTTON", Some(sq(10.0, 20.0)))
            .with_parent("panel")
            .with_text("查看 2 条回复");
        trigger
            .click_errors
            .push_back(DoudouError::new("TIMEOUT", "blocked 1"));
        trigger
            .click_errors
            .push_back(DoudouError::new("TIMEOUT", "blocked 2"));
        s.add_element(trigger);
        s.bind_css(None, r#"[data-e2e="comment-list"]"#, ["panel"]);
        s.bind_css(
            Some("panel"),
            VideoCommentsPanelComponent::REPLY_EXPAND_SELECTOR,
            ["trigger"],
        );
    });

    let err = VideoCommentsPanelComponent
        .trigger_reply_template(&page, 2)
        .await
        .expect_err("must stop at retry limit");
    assert_eq!(err.code, "PAGE_ADAPTER_DRIFTED");
    let details = err.details.expect("details present");
    assert_eq!(details["attempts"], 2);
    assert_eq!(details["geometry_reason"], "reply_trigger_action_failed");
}

#[tokio::test]
async fn reply_template_scrolls_before_tolerant_scope_validation() {
    let page = MockUiPage::new();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "panel",
            "DIV",
            Some(bbox(100.0, 100.0, 100.0, 40.0)),
        ));
        // Pre-scroll geometry is 2.1px beyond panel; after scrolling it fits inside scope.
        let trigger = MockElement::new("trigger", "BUTTON", Some(sq(182.1, 100.0)))
            .with_parent("panel")
            .with_text("展开 2 条回复")
            .with_after_scroll_box(sq(180.0, 100.0));
        s.add_element(trigger);
        s.bind_css(None, r#"[data-e2e="comment-list"]"#, ["panel"]);
        s.bind_css(
            Some("panel"),
            VideoCommentsPanelComponent::REPLY_EXPAND_SELECTOR,
            ["trigger"],
        );
    });

    let result = VideoCommentsPanelComponent
        .trigger_reply_template(&page, 2)
        .await
        .expect("scrolled trigger succeeds");
    assert_eq!(result, 1);
    let (clicks, events) = page.with_state(|s| {
        (
            s.elements[&ElementId::from("trigger")].click_calls,
            s.events.clone(),
        )
    });
    assert_eq!(clicks, 1);
    let scroll_idx = events
        .iter()
        .position(|e| e == "scroll:trigger")
        .expect("scroll recorded");
    let post_scroll_box_idx = events
        .iter()
        .rposition(|e| e == "box:trigger")
        .expect("box recorded after scroll");
    assert!(scroll_idx < post_scroll_box_idx);
}

#[tokio::test]
async fn reply_template_rejects_target_more_than_tolerance_outside_scope() {
    let page = MockUiPage::new();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "panel",
            "DIV",
            Some(bbox(100.0, 100.0, 100.0, 40.0)),
        ));
        let trigger = MockElement::new("trigger", "BUTTON", Some(sq(185.0, 100.0)))
            .with_parent("panel")
            .with_text("展开 2 条回复")
            .with_after_scroll_box(sq(185.0, 100.0));
        s.add_element(trigger);
        s.bind_css(None, r#"[data-e2e="comment-list"]"#, ["panel"]);
        s.bind_css(
            Some("panel"),
            VideoCommentsPanelComponent::REPLY_EXPAND_SELECTOR,
            ["trigger"],
        );
    });

    let err = VideoCommentsPanelComponent
        .trigger_reply_template(&page, 2)
        .await
        .expect_err("must reject trigger beyond 4px tolerance");
    assert_eq!(err.code, "PAGE_ADAPTER_DRIFTED");
    assert_eq!(
        err.details
            .as_ref()
            .and_then(|d| d["geometry_reason"].as_str()),
        Some("outside_local_scope")
    );
    assert_eq!(
        page.with_state(|s| s.elements[&ElementId::from("trigger")].click_calls),
        0
    );
}

#[test]
fn message_semantics_matches_rendered_emoji_titles() {
    assert!(message_semantics_match(
        "正文[尬笑]",
        &MessageEvidence {
            text: "正文".to_owned(),
            media_labels: vec!["[尬笑]".to_owned()],
        }
    ));
    assert!(!message_semantics_match(
        "正文[微笑]",
        &MessageEvidence {
            text: "正文".to_owned(),
            media_labels: vec!["[尬笑]".to_owned()],
        }
    ));
    assert!(!message_semantics_match(
        "正文",
        &MessageEvidence {
            text: "正文".to_owned(),
            media_labels: vec!["[尬笑]".to_owned()],
        }
    ));
}

#[tokio::test]
async fn unique_enabled_target_inside_scope_and_rightmost_passes() {
    let page = MockUiPage::new();
    let (scope, left, target) = page.with_state(|s| {
        let scope = s.add_element(MockElement::new(
            "scope",
            "DIV",
            Some(bbox(0.0, 0.0, 300.0, 100.0)),
        ));
        let left = s.add_element(MockElement::new("left", "SVG", Some(sq(210.0, 40.0))));
        let target = s.add_element(MockElement::new("target", "SVG", Some(sq(260.0, 40.0))));
        (scope, left, target)
    });
    let peers = [left, target.clone()];
    let resolved = require_target(
        &page,
        std::slice::from_ref(&target),
        &ComponentContext::new(
            "chat-composer",
            "test.v1",
            "chat.send",
            "local semantic send -> rightmost action",
        ),
        &TargetOptions::default()
            .require_enabled()
            .scope(&scope)
            .peers(&peers, PeerPosition::Rightmost, 2),
    )
    .await
    .expect("target resolves");
    assert_eq!(resolved, target);
}

#[tokio::test]
async fn zero_and_duplicate_candidates_fail_closed() {
    let page = MockUiPage::new();
    let empty = assert_drift(&page, &[], "target_not_unique", &TargetOptions::default()).await;
    assert_eq!(empty.details.unwrap()["candidate_count"], 0);

    let (a, b) = page.with_state(|s| {
        (
            s.add_element(MockElement::new("a", "DIV", Some(sq(1.0, 1.0)))),
            s.add_element(MockElement::new("b", "DIV", Some(sq(2.0, 2.0)))),
        )
    });
    let dup = assert_drift(
        &page,
        &[a, b],
        "target_not_unique",
        &TargetOptions::default(),
    )
    .await;
    assert_eq!(dup.details.unwrap()["visible_count"], 2);
}

#[tokio::test]
async fn hidden_and_disabled_targets_fail_closed() {
    let page = MockUiPage::new();
    let (hidden, disabled) = page.with_state(|s| {
        (
            s.add_element(
                MockElement::new("hidden", "DIV", Some(sq(1.0, 1.0))).with_visible(false),
            ),
            s.add_element(
                MockElement::new("disabled", "BUTTON", Some(sq(1.0, 1.0))).with_enabled(false),
            ),
        )
    });
    let _ = assert_drift(
        &page,
        &[hidden],
        "target_not_unique",
        &TargetOptions::default(),
    )
    .await;
    let err = assert_drift(
        &page,
        &[disabled],
        "target_disabled",
        &TargetOptions::default().require_enabled(),
    )
    .await;
    assert_eq!(err.details.unwrap()["enabled_count"], 0);
}

#[tokio::test]
async fn target_outside_local_scope_fails_closed() {
    let page = MockUiPage::new();
    let (scope, outside) = page.with_state(|s| {
        (
            s.add_element(MockElement::new(
                "scope",
                "DIV",
                Some(bbox(0.0, 0.0, 300.0, 100.0)),
            )),
            s.add_element(MockElement::new("outside", "DIV", Some(sq(400.0, 20.0)))),
        )
    });
    let _ = assert_drift(
        &page,
        &[outside],
        "outside_local_scope",
        &TargetOptions::default().scope(&scope),
    )
    .await;
}

#[tokio::test]
async fn component_specific_scope_tolerance_remains_bounded() {
    let page = MockUiPage::new();
    let (scope, ok_overflow, too_far) = page.with_state(|s| {
        (
            s.add_element(MockElement::new(
                "scope",
                "DIV",
                Some(bbox(214.0, 100.0, 100.0, 30.0)),
            )),
            s.add_element(MockElement::new(
                "ok_overflow",
                "DIV",
                Some(bbox(211.0, 100.0, 100.0, 20.0)),
            )),
            s.add_element(MockElement::new(
                "too_far",
                "DIV",
                Some(bbox(209.0, 100.0, 100.0, 20.0)),
            )),
        )
    });

    let resolved = require_target(
        &page,
        std::slice::from_ref(&ok_overflow),
        &ComponentContext::new(
            "video-comment-item",
            "test.v1",
            "video.comment.like",
            "comment-item-local stats container",
        ),
        &TargetOptions::default().scope(&scope).scope_tolerance(4.0),
    )
    .await
    .expect("within 4px tolerance");
    assert_eq!(resolved, ok_overflow);

    let _ = assert_drift(
        &page,
        &[too_far],
        "outside_local_scope",
        &TargetOptions::default().scope(&scope).scope_tolerance(4.0),
    )
    .await;
}

#[tokio::test]
async fn wrong_relative_position_fails_closed() {
    let page = MockUiPage::new();
    let (target, peer_right) = page.with_state(|s| {
        (
            s.add_element(MockElement::new("target", "SVG", Some(sq(200.0, 20.0)))),
            s.add_element(MockElement::new("right", "SVG", Some(sq(260.0, 20.0)))),
        )
    });
    let peers = [target.clone(), peer_right];
    let _ = assert_drift(
        &page,
        &[target],
        "not_rightmost",
        &TargetOptions::default().peers(&peers, PeerPosition::Rightmost, 2),
    )
    .await;
}

#[tokio::test]
async fn anchor_filter_selects_only_local_candidate() {
    let page = MockUiPage::new();
    let (anchor, local, unrelated) = page.with_state(|s| {
        (
            s.add_element(MockElement::new(
                "anchor",
                "DIV",
                Some(bbox(200.0, 500.0, 400.0, 50.0)),
            )),
            s.add_element(MockElement::new(
                "local",
                "DIV",
                Some(bbox(220.0, 400.0, 120.0, 30.0)),
            )),
            s.add_element(MockElement::new(
                "unrelated",
                "DIV",
                Some(bbox(40.0, 40.0, 120.0, 30.0)),
            )),
        )
    });
    let resolved = require_target(
        &page,
        &[unrelated, local.clone()],
        &fixture_ctx(),
        &TargetOptions::default()
            .anchor(&anchor, Some(AnchorRelation::Above), Some(300.0))
            .filter_by_geometry(),
    )
    .await
    .expect("local candidate selected");
    assert_eq!(resolved, local);
}

#[tokio::test]
async fn probe_reports_geometry_drift() {
    let page = MockUiPage::new();
    let missing_box = page.with_state(|s| s.add_element(MockElement::new("nobox", "DIV", None)));
    let report = component_probe(&page, &[missing_box], &fixture_ctx(), true)
        .await
        .expect("probe runs");
    assert_eq!(report["status"], "drifted");
    assert_eq!(report["geometry_valid"], false);
    assert_eq!(report["geometry_reason"], "target_box_missing");
}

#[tokio::test]
async fn verified_mouse_click_uses_stable_dynamic_center() {
    let page = MockUiPage::new();
    let target = page.with_state(|s| {
        s.add_element(
            MockElement::new("row", "DIV", Some(bbox(100.0, 200.0, 80.0, 40.0))).with_box_sequence(
                [
                    Some(bbox(100.0, 200.0, 80.0, 40.0)),
                    Some(bbox(100.0, 200.0, 80.0, 40.0)),
                ],
            ),
        )
    });
    let evidence = click_verified_target(
        &page,
        &target,
        &ComponentContext::new(
            "conversation-list",
            "test.v1",
            "chat.conversation.open",
            "exact row -> stable hit target -> dynamic mouse click",
        ),
        ClickOptions::default(),
    )
    .await
    .expect("verified click succeeds");
    assert_eq!(page.with_state(|s| s.elements[&target].scroll_calls), 1);
    let clicks = page.with_state(|s| s.mouse_clicks.clone());
    assert_eq!(clicks.len(), 1);
    assert!((clicks[0].x - 140.0).abs() < f64::EPSILON);
    assert!((clicks[0].y - 220.0).abs() < f64::EPSILON);
    assert!(evidence.stable);
    assert!(evidence.receives_events);
    assert!(evidence.dispatch_completed);
}

#[tokio::test]
async fn verified_mouse_click_can_skip_redundant_actionability_scroll() {
    let page = MockUiPage::new();
    let target = page.with_state(|s| {
        s.add_element(MockElement::new(
            "row",
            "DIV",
            Some(bbox(100.0, 200.0, 80.0, 40.0)),
        ))
    });
    let _ = click_verified_target(
        &page,
        &target,
        &fixture_ctx(),
        ClickOptions::default().without_scroll(),
    )
    .await
    .expect("click succeeds");
    assert_eq!(page.with_state(|s| s.elements[&target].scroll_calls), 0);
    let clicks = page.with_state(|s| s.mouse_clicks.clone());
    assert_eq!(clicks.len(), 1);
    assert!((clicks[0].x - 140.0).abs() < f64::EPSILON);
    assert!((clicks[0].y - 220.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn verified_mouse_click_defers_timeout_to_state_postcondition() {
    let page = MockUiPage::new();
    let target = page.with_state(|s| {
        s.click_xy_error = Some(DoudouError::new("CLICK_TIMEOUT", "click timed out"));
        s.add_element(MockElement::new(
            "row",
            "DIV",
            Some(bbox(100.0, 200.0, 80.0, 40.0)),
        ))
    });
    let evidence = click_verified_target(
        &page,
        &target,
        &fixture_ctx(),
        ClickOptions::default().without_scroll(),
    )
    .await
    .expect("timeout deferred to postcondition");
    assert!(!evidence.dispatch_completed);
}

#[tokio::test]
async fn verified_mouse_click_fails_closed_when_occluded() {
    let page = MockUiPage::new();
    let target = page.with_state(|s| {
        s.hit_override = Some(Some(PointHit {
            element_id: Some(ElementId::from("mask")),
            tag: "DIV".to_owned(),
            class_name: "loading-mask".to_owned(),
            ancestor_ids: vec![ElementId::from("mask"), ElementId::from("body")],
            non_interactive: false,
        }));
        s.add_element(MockElement::new(
            "row",
            "DIV",
            Some(bbox(100.0, 200.0, 80.0, 40.0)),
        ))
    });
    let err = click_verified_target(&page, &target, &fixture_ctx(), ClickOptions::default())
        .await
        .expect_err("occluded target must fail closed");
    assert_eq!(err.code, "PAGE_ADAPTER_DRIFTED");
    assert_eq!(
        err.details
            .as_ref()
            .and_then(|d| d["geometry_reason"].as_str()),
        Some("target_occluded")
    );
    assert!(page.with_state(|s| s.mouse_clicks.is_empty()));
}

#[tokio::test]
async fn chat_composer_focuses_with_mouse_and_sends_with_enter() {
    let page = MockUiPage::new();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "input-row",
            "DIV",
            Some(bbox(320.0, 700.0, 900.0, 80.0)),
        ));
        s.add_element(
            MockElement::new("editor", "DIV", Some(bbox(340.0, 710.0, 600.0, 40.0)))
                .with_parent("input-row")
                .with_text("draft"),
        );
        s.add_element(
            MockElement::new("emoji-svg", "SVG", Some(sq(1000.0, 720.0))).with_parent("input-row"),
        );
        s.add_element(
            MockElement::new("send-svg", "SVG", Some(sq(1160.0, 720.0))).with_parent("input-row"),
        );
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
    });

    let composer = ChatComposerComponent::new(&page);
    let _ = composer.clear().await.expect("clear succeeds");
    let ops = composer.send().await.expect("send succeeds");

    assert_eq!(ops, 1);
    assert_eq!(page.with_state(|s| s.mouse_clicks.len()), 1);
    let keys = page.with_state(|s| s.elements[&ElementId::from("editor")].key_presses.clone());
    assert_eq!(keys, vec!["Control+A", "Backspace", "Enter"]);
}

#[tokio::test]
async fn video_comment_validates_control_and_sends_with_enter() {
    let page = MockUiPage::new();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "container",
            "DIV",
            Some(bbox(700.0, 700.0, 500.0, 60.0)),
        ));
        s.add_element(
            MockElement::new("editor", "DIV", Some(bbox(710.0, 710.0, 340.0, 30.0)))
                .with_parent("container"),
        );
        s.add_element(
            MockElement::new("emoji-btn", "SPAN", Some(sq(1100.0, 715.0))).with_parent("container"),
        );
        s.add_element(
            MockElement::new("send-btn", "SPAN", Some(sq(1160.0, 715.0))).with_parent("container"),
        );
        s.bind_css(
            None,
            VideoCommentComposerComponent::CONTAINER_SELECTOR,
            ["container"],
        );
        s.bind_css(
            Some("container"),
            VideoCommentComposerComponent::EDITOR_SELECTOR,
            ["editor"],
        );
        s.bind_css(
            Some("container"),
            VideoCommentComposerComponent::CONTROLS_SELECTOR,
            ["emoji-btn", "send-btn"],
        );
    });

    let composer = VideoCommentComposerComponent::new(&page);
    let ops = composer.publish().await.expect("publish succeeds");
    assert_eq!(ops, 1);
    assert_eq!(
        page.with_state(|s| s.elements[&ElementId::from("editor")].key_presses.clone()),
        vec!["Enter"]
    );
    assert_eq!(
        page.with_state(|s| s.elements[&ElementId::from("send-btn")].click_calls),
        0
    );
}

#[tokio::test]
async fn video_reply_composer_is_scoped_to_the_confirmed_comment() {
    let page = MockUiPage::new();
    let comment_item = page.with_state(|s| {
        let item = s.add_element(MockElement::new(
            "comment-1",
            "DIV",
            Some(bbox(700.0, 200.0, 500.0, 120.0)),
        ));
        s.add_element(
            MockElement::new("reply-box", "DIV", Some(bbox(710.0, 260.0, 480.0, 40.0)))
                .with_parent("comment-1"),
        );
        s.add_element(MockElement::new(
            "top-level-box",
            "DIV",
            Some(bbox(700.0, 700.0, 500.0, 40.0)),
        ));
        s.bind_css(
            Some("comment-1"),
            VideoCommentComposerComponent::CONTAINER_SELECTOR,
            ["reply-box"],
        );
        s.bind_css(
            None,
            VideoCommentComposerComponent::CONTAINER_SELECTOR,
            ["top-level-box"],
        );
        item
    });

    let composer = VideoCommentComposerComponent::scoped(&page, comment_item);
    assert_eq!(composer.capability(), "video.comment.reply");
    let found = composer.candidates().await.expect("scoped lookup");
    assert_eq!(found, vec![ElementId::from("reply-box")]);
}

#[tokio::test]
async fn video_comment_focus_uses_mouse_postcondition_once() {
    let page = MockUiPage::new();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "container",
            "DIV",
            Some(bbox(700.0, 700.0, 500.0, 60.0)),
        ));
        s.add_element(
            MockElement::new("editor", "DIV", Some(bbox(710.0, 710.0, 340.0, 30.0)))
                .with_parent("container")
                .with_visible(false),
        );
        s.bind_css(
            None,
            VideoCommentComposerComponent::CONTAINER_SELECTOR,
            ["container"],
        );
        s.bind_css(
            Some("container"),
            VideoCommentComposerComponent::EDITOR_SELECTOR,
            ["editor"],
        );
    });
    page.set_point_click_hook(|s, _x, _y, _button| {
        if let Some(editor) = s.elements.get_mut(&ElementId::from("editor")) {
            editor.visible = true;
            editor.focused = true;
        }
        Ok(())
    });

    let composer = VideoCommentComposerComponent::new(&page);
    let _ = composer.activate().await.expect("activate succeeds");
    let editor = composer
        .insert_text("测试评论")
        .await
        .expect("insert_text succeeds");
    assert_eq!(editor, ElementId::from("editor"));
    assert_eq!(page.with_state(|s| s.mouse_clicks.len()), 1);
    assert_eq!(
        page.with_state(|s| s.inserted_texts.clone()),
        vec!["测试评论"]
    );
}

#[tokio::test]
async fn video_comment_focus_fails_closed_without_postcondition() {
    let page = MockUiPage::new();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "container",
            "DIV",
            Some(bbox(700.0, 700.0, 500.0, 60.0)),
        ));
        s.add_element(
            MockElement::new("editor", "DIV", Some(bbox(710.0, 710.0, 340.0, 30.0)))
                .with_parent("container"),
        );
        s.bind_css(
            None,
            VideoCommentComposerComponent::CONTAINER_SELECTOR,
            ["container"],
        );
        s.bind_css(
            Some("container"),
            VideoCommentComposerComponent::EDITOR_SELECTOR,
            ["editor"],
        );
    });
    // Click does NOT set editor.focused = true.
    page.set_point_click_hook(|_s, _x, _y, _button| Ok(()));

    let err = VideoCommentComposerComponent::new(&page)
        .activate()
        .await
        .expect_err("must fail closed when focus postcondition fails");
    assert_eq!(err.code, "PAGE_ADAPTER_DRIFTED");
    assert_eq!(
        err.details
            .as_ref()
            .and_then(|d| d["geometry_reason"].as_str()),
        Some("comment_editor_not_focused_after_action")
    );
}

// ---------------------------------------------------------------------------
// PR #8 ConversationListComponent tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conversation_list_waits_for_unmounted_container_then_resets_scroll() {
    let page = MockUiPage::new();
    let scroll_top = Arc::new(AtomicUsize::new(240));
    let reset_calls = Arc::new(AtomicUsize::new(0));
    let lookups = Arc::new(AtomicUsize::new(0));

    page.with_state(|s| {
        s.add_element(MockElement::new(
            "conv-list",
            "DIV",
            Some(bbox(0.0, 100.0, 320.0, 400.0)),
        ));
        s.add_element(
            MockElement::new("row-target", "DIV", Some(bbox(20.0, 120.0, 280.0, 60.0)))
                .with_parent("conv-list")
                .with_class("conversationConversationItem")
                .with_text("测试群聊")
                .with_visible(false),
        );
        s.add_element(
            MockElement::new("row-old", "DIV", Some(bbox(20.0, 120.0, 280.0, 60.0)))
                .with_parent("conv-list")
                .with_class("conversationConversationItem")
                .with_text("旧会话")
                .with_visible(true),
        );
    });

    let conv_list_sel = CHAT_ADAPTER
        .selector("conversation_list")
        .unwrap()
        .to_owned();
    let session_sel = CHAT_ADAPTER
        .selector("session_candidates")
        .unwrap()
        .to_owned();
    let lookups_q = Arc::clone(&lookups);
    let scroll_q = Arc::clone(&scroll_top);
    page.set_query_override(move |state, scope, query| match (scope, query) {
        (None, SelectorQuery::Css(sel)) if *sel == conv_list_sel => {
            let n = lookups_q.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                Some(Vec::new())
            } else {
                Some(vec![ElementId::from("conv-list")])
            }
        }
        (Some(parent), SelectorQuery::Css(sel))
            if parent.as_str() == "conv-list" && *sel == session_sel =>
        {
            if scroll_q.load(Ordering::SeqCst) == 0 {
                if let Some(r) = state.elements.get_mut(&ElementId::from("row-target")) {
                    r.visible = true;
                }
                if let Some(r) = state.elements.get_mut(&ElementId::from("row-old")) {
                    r.visible = false;
                }
                Some(vec![ElementId::from("row-target")])
            } else {
                Some(vec![ElementId::from("row-old")])
            }
        }
        _ => None,
    });

    let scroll_e = Arc::clone(&scroll_top);
    let reset_e = Arc::clone(&reset_calls);
    page.set_eval_hook(move |_elem, _script, arg| {
        let reset = arg.and_then(Value::as_bool).unwrap_or(false);
        let before = scroll_e.load(Ordering::SeqCst);
        let after = if reset {
            reset_e.fetch_add(1, Ordering::SeqCst);
            scroll_e.store(0, Ordering::SeqCst);
            0
        } else {
            before
        };
        Ok(json!({
            "resolved": true,
            "before": before,
            "after": after,
            "client_height": 400,
            "scroll_height": 800,
            "moved": after != before,
        }))
    });

    page.set_point_click_hook(|s, x, y, _btn| {
        if let Some(row) = s.elements.get_mut(&ElementId::from("row-target"))
            && row.last_box.is_some_and(|b| b.contains_point(x, y))
        {
            row.classes
                .push(ConversationListComponent::SELECTED_CLASS.to_owned());
        }
        Ok(())
    });

    let ops = ConversationListComponent::new(&page)
        .open("测试群聊")
        .await
        .expect("waits for mount, resets scroll, and selects target");

    assert_eq!(ops, 1);
    assert_eq!(reset_calls.load(Ordering::SeqCst), 1);
    let clicks = page.with_state(|s| s.mouse_clicks.clone());
    assert_eq!(clicks.len(), 1);
    assert!((clicks[0].x - 160.0).abs() < f64::EPSILON);
    assert!((clicks[0].y - 150.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn conversation_list_fails_immediately_on_non_empty_container_drift() {
    let page = MockUiPage::new();
    let conv_list_sel = CHAT_ADAPTER.selector("conversation_list").unwrap();
    page.with_state(|s| {
        s.add_element(MockElement::new(
            "c1",
            "DIV",
            Some(bbox(0.0, 100.0, 320.0, 400.0)),
        ));
        s.add_element(MockElement::new(
            "c2",
            "DIV",
            Some(bbox(340.0, 100.0, 320.0, 400.0)),
        ));
        s.bind_css(None, conv_list_sel, ["c1", "c2"]);
    });

    let err = ConversationListComponent::new(&page)
        .open("测试群聊")
        .await
        .expect_err("duplicate containers must fail immediately");
    assert_eq!(err.code, "PAGE_ADAPTER_DRIFTED");
    let details = err.details.expect("details present");
    assert_eq!(details["candidate_count"], 2);
    assert_eq!(details["visible_count"], 2);
    assert!(page.with_state(|s| s.waits_ms.is_empty()));
}

#[tokio::test]
async fn conversation_list_scrolls_partially_clipped_row_into_scope() {
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
            MockElement::new("row-clipped", "DIV", Some(bbox(20.0, 470.0, 280.0, 60.0)))
                .with_parent("conv-list")
                .with_class("conversationConversationItem")
                .with_text("测试私聊")
                .with_after_scroll_box(bbox(20.0, 440.0, 280.0, 60.0)),
        );
        s.bind_css(None, conv_list_sel, ["conv-list"]);
        s.bind_css(Some("conv-list"), session_sel, ["row-clipped"]);
    });
    page.set_eval_hook(|_e, _s, _a| Ok(json!({ "resolved": true, "moved": false })));
    page.set_point_click_hook(|s, x, y, _btn| {
        if let Some(row) = s.elements.get_mut(&ElementId::from("row-clipped"))
            && row.last_box.is_some_and(|b| b.contains_point(x, y))
        {
            row.classes
                .push(ConversationListComponent::SELECTED_CLASS.to_owned());
        }
        Ok(())
    });

    let ops = ConversationListComponent::new(&page)
        .open("测试私聊")
        .await
        .expect("clipped row scrolled into scope and clicked");
    assert_eq!(ops, 1);
    assert_eq!(
        page.with_state(|s| s.elements[&ElementId::from("row-clipped")].scroll_calls),
        1
    );
    let clicks = page.with_state(|s| s.mouse_clicks.clone());
    assert_eq!(clicks.len(), 1);
    assert!((clicks[0].x - 160.0).abs() < f64::EPSILON);
    assert!((clicks[0].y - 470.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn conversation_list_rejects_row_outside_scope_after_scroll() {
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
            MockElement::new("row-outside", "DIV", Some(bbox(20.0, 520.0, 280.0, 60.0)))
                .with_parent("conv-list")
                .with_class("conversationConversationItem")
                .with_text("越界会话")
                .with_after_scroll_box(bbox(20.0, 520.0, 280.0, 60.0)),
        );
        s.bind_css(None, conv_list_sel, ["conv-list"]);
        s.bind_css(Some("conv-list"), session_sel, ["row-outside"]);
    });
    page.set_eval_hook(|_e, _s, _a| Ok(json!({ "resolved": true, "moved": false })));

    let err = ConversationListComponent::new(&page)
        .open("越界会话")
        .await
        .expect_err("row still outside scope must fail closed");
    assert_eq!(err.code, "PAGE_ADAPTER_DRIFTED");
    assert_eq!(
        err.details
            .as_ref()
            .and_then(|d| d["geometry_reason"].as_str()),
        Some("outside_local_scope")
    );
    assert!(page.with_state(|s| s.mouse_clicks.is_empty()));
}

#[tokio::test]
async fn conversation_list_open_skips_decorative_icons_and_uses_ancestor_chain_containment() {
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
                .with_class("conversationConversationItem"),
        );
        // Real visible label child inside row-1.
        s.add_element(
            MockElement::new("row-1-label", "SPAN", Some(bbox(80.0, 130.0, 160.0, 24.0)))
                .with_parent("row-1")
                .with_text("产品交流群"),
        );
        // Decorative duplicate icon/badge inside row-1 (aria-hidden="true") that sits over center.
        s.add_element(
            MockElement::new("row-1-deco", "SVG", Some(bbox(140.0, 135.0, 40.0, 30.0)))
                .with_parent("row-1")
                .with_attr("aria-hidden", "true")
                .with_text("产品交流群")
                .with_decorative(true),
        );
        s.bind_css(None, conv_list_sel, ["conv-list"]);
        s.bind_css(Some("conv-list"), session_sel, ["row-1"]);
    });
    page.set_eval_hook(|_e, _s, _a| Ok(json!({ "resolved": true, "moved": false })));
    page.set_point_click_hook(|s, x, y, _btn| {
        if let Some(row) = s.elements.get_mut(&ElementId::from("row-1"))
            && row.last_box.is_some_and(|b| b.contains_point(x, y))
        {
            row.classes
                .push(ConversationListComponent::SELECTED_CLASS.to_owned());
        }
        Ok(())
    });

    // Center of row-1 is (160, 150) which hits child `row-1-deco`.
    // Because `row-1-deco` has `row-1` in its ancestor chain, `element_from_point` passes
    // containment check, AND the decorative duplicate text is skipped so label matching stays unique!
    let ops = ConversationListComponent::new(&page)
        .open("产品交流群")
        .await
        .expect("ancestor-chain hit succeeds and decorative duplicate is skipped");
    assert_eq!(ops, 1);
}

#[tokio::test]
async fn conversation_list_open_blocks_false_click_when_occluded_by_overlapping_header_tab() {
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
                .with_class("conversationConversationItem")
                .with_text("重要会话"),
        );
        // Overlapping sticky header / tab bar rendered AFTER row-1 (outside row-1's ancestor chain).
        s.add_element(
            MockElement::new("sticky-tab", "HEADER", Some(bbox(0.0, 100.0, 320.0, 90.0)))
                .with_parent("body")
                .with_class("chat-top-tabs"),
        );
        s.bind_css(None, conv_list_sel, ["conv-list"]);
        s.bind_css(Some("conv-list"), session_sel, ["row-1"]);
    });
    page.set_eval_hook(|_e, _s, _a| Ok(json!({ "resolved": true, "moved": false })));

    let err = ConversationListComponent::new(&page)
        .open("重要会话")
        .await
        .expect_err("overlapping tab must prevent false click");
    assert_eq!(err.code, "PAGE_ADAPTER_DRIFTED");
    let details = err.details.expect("details present");
    assert_eq!(details["geometry_reason"], "target_occluded");
    assert_eq!(details["blocker"]["tag"], "HEADER");
    assert_eq!(details["blocker"]["class_name"], "chat-top-tabs");
    assert!(page.with_state(|s| s.mouse_clicks.is_empty()));
}
