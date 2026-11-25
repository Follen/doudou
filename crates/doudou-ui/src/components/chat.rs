//! Douyin chat page components: conversation list, composer, message items, and image upload.

use std::path::{Path, PathBuf};

use doudou_core::{DoudouError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::adapters::CHAT_ADAPTER;
use crate::geometry::{
    ClickOptions, ComponentContext, DEFAULT_CANDIDATE_LIMIT, PeerPosition, TargetOptions,
    click_verified_target, component_probe, drift_error, not_exercised_probe, require_target,
    resolution_probe, unique_visible, visible_candidates,
};
use crate::page::{ElementId, MouseButton, SelectorQuery, TextMatchMode, UiPage};

const ZERO_WIDTH_SENTINELS: [char; 4] = ['\u{200b}', '\u{200c}', '\u{200d}', '\u{feff}'];

const SCROLL_LOCAL_VIEWPORT_JS: &str = r#"(element, reset) => {
  const first = element.querySelector('[data-e2e="conversation-item"]');
  if (!first) return {resolved: false, reason: 'no_session_item'};
  const candidates = [];
  let current = first.parentElement;
  while (current && element.contains(current)) {
    if (current.scrollHeight > current.clientHeight + 1) candidates.push(current);
    if (current === element) break;
    current = current.parentElement;
  }
  if (candidates.length !== 1) {
    return {resolved: false, reason: 'scroll_viewport_not_unique', candidate_count: candidates.length};
  }
  const viewport = candidates[0];
  const before = viewport.scrollTop;
  viewport.scrollTop = reset
    ? 0
    : Math.min(viewport.scrollHeight, before + Math.max(160, viewport.clientHeight * 0.8));
  return {
    resolved: true,
    before,
    after: viewport.scrollTop,
    client_height: viewport.clientHeight,
    scroll_height: viewport.scrollHeight,
    moved: viewport.scrollTop !== before,
  };
}"#;

const ACTIVE_ELEMENT_JS: &str =
    "element => element === document.activeElement || element.contains(document.activeElement)";

const MESSAGE_EVIDENCE_JS: &str = r"element => ({
  text: element.innerText || element.textContent || '',
  media_labels: [...element.querySelectorAll('img')]
    .map(image => image.getAttribute('title') || image.getAttribute('alt'))
    .filter(Boolean),
})";

fn normalize_rendered_text(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .trim_end_matches('\n')
        .chars()
        .filter(|ch| !ZERO_WIDTH_SENTINELS.contains(ch))
        .collect()
}

/// Rendered text and emoji media labels extracted from a chat message element.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageEvidence {
    pub text: String,
    #[serde(default)]
    pub media_labels: Vec<String>,
}

/// Check whether a rendered chat message's text plus emoji `img` labels match protocol `expected`.
pub fn message_semantics_match(expected: &str, evidence: &MessageEvidence) -> bool {
    let mut remaining = expected.to_owned();
    for label in &evidence.media_labels {
        if label.is_empty() || !remaining.contains(label.as_str()) {
            return false;
        }
        remaining = remaining.replacen(label.as_str(), "", 1);
    }
    normalize_rendered_text(&evidence.text) == normalize_rendered_text(&remaining)
}

async fn page_body(page: &dyn UiPage) -> Result<Option<ElementId>> {
    let nodes = page.query_all(None, &SelectorQuery::css("body"), 1).await?;
    Ok(nodes.into_iter().next())
}

/// Left-hand conversation list component (`conversation-list`, version `2026-08-03.3`).
pub struct ConversationListComponent<'a> {
    page: &'a dyn UiPage,
}

impl<'a> ConversationListComponent<'a> {
    pub const NAME: &'static str = "conversation-list";
    pub const VERSION: &'static str = "2026-08-03.3";
    pub const SELECTED_CLASS: &'static str = "conversationConversationItemcurConversation";

    /// Construct a conversation list component bound to `page`.
    pub const fn new(page: &'a dyn UiPage) -> Self {
        Self { page }
    }

    async fn container(&self) -> Result<ElementId> {
        let candidates = CHAT_ADAPTER
            .query_selector(self.page, "conversation_list", DEFAULT_CANDIDATE_LIMIT)
            .await?;
        let body = page_body(self.page).await?;
        let mut opts = TargetOptions::default();
        if let Some(ref b) = body {
            opts = opts.scope(b);
        }
        require_target(
            self.page,
            &candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.conversation.list",
                "versioned conversation-list container with data-e2e items",
            ),
            &opts,
        )
        .await
    }

    async fn items(&self) -> Result<(ElementId, Vec<ElementId>, usize)> {
        let container = self.container().await?;
        let selector = CHAT_ADAPTER
            .selector("session_candidates")
            .unwrap_or(r#"[data-e2e="conversation-item"]"#);
        let raw = self
            .page
            .query_all(Some(&container), &SelectorQuery::css(selector), 100)
            .await?;
        let total = raw.len().min(100);
        let visible = visible_candidates(self.page, &raw, DEFAULT_CANDIDATE_LIMIT).await?;
        let mut interactive = Vec::with_capacity(visible.len());
        for item in visible {
            if !self.page.is_decorative(&item).await? {
                interactive.push(item);
            }
        }
        Ok((container, interactive, total))
    }

    async fn scroll_local_viewport(&self, container: &ElementId, reset: bool) -> Result<Value> {
        self.page
            .eval_on_element(container, SCROLL_LOCAL_VIEWPORT_JS, Some(&json!(reset)))
            .await
    }

    /// Collect bounded evidence describing whether the chat conversation list has mounted.
    pub async fn evidence(&self, wait_ms: u64) -> Result<Value> {
        let deadline = self.page.now_ms().saturating_add(wait_ms);
        let (items_count, total, loaded, container_count) = loop {
            let state = match self.items().await {
                Ok((_container, items, total)) => (items.len(), total, !items.is_empty(), 1usize),
                Err(_) => (0, 0, false, 0usize),
            };
            if state.2 || self.page.now_ms() >= deadline {
                break state;
            }
            self.page.sleep_ms(250).await?;
        };
        let url = self.page.url().await?;
        Ok(json!({
            "chat_route": url.starts_with("https://www.douyin.com/chat"),
            "loaded": loaded,
            "candidate_count": total,
            "left_region_count": items_count,
            "container_count": container_count,
            "component": Self::NAME,
            "component_version": Self::VERSION,
            "strategy": "unique versioned conversation list -> local data-e2e items",
        }))
    }

    /// Open the conversation row matching `display_name` exactly, returning DOM click count (`0` or `1`).
    ///
    /// Includes the PR #8 fixes:
    /// - Waits up to 15s for an unmounted container before resetting scroll, while failing
    ///   immediately on non-empty container drift.
    /// - Skips non-interactive decorative elements when matching exact row labels.
    /// - Scrolls a matched row into view before validating containment within `container`.
    /// - Uses `click_verified_target` (stability + `elementFromPoint` ancestor-chain containment)
    ///   so overlapping tabs, headers, or masks cannot trigger false clicks.
    pub async fn open(&self, display_name: &str) -> Result<usize> {
        let deadline = self.page.now_ms().saturating_add(15_000);
        let mut target: Option<ElementId> = None;
        let mut diagnostics = json!({ "candidate_count": 0, "visible_count": 0 });
        let mut scroll = json!({ "resolved": false, "moved": false });
        let mut reset_done = false;

        while self.page.now_ms() < deadline {
            let (container, items, total_items) = match self.items().await {
                Ok(found) => found,
                Err(error) => {
                    if error.code != "PAGE_ADAPTER_DRIFTED" {
                        return Err(error);
                    }
                    let details = error.details.as_ref();
                    let cand_count = details
                        .and_then(|d| d.get("candidate_count"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let vis_count = details
                        .and_then(|d| d.get("visible_count"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    if cand_count > 0 || vis_count > 0 {
                        return Err(error);
                    }
                    diagnostics = json!({
                        "candidate_count": 0,
                        "visible_count": 0,
                        "matching_items": 0,
                        "render_state": "conversation_list_pending",
                    });
                    self.page.sleep_ms(250).await?;
                    continue;
                }
            };

            if !reset_done {
                reset_done = true;
                scroll = self.scroll_local_viewport(&container, true).await?;
                let resolved = scroll
                    .get("resolved")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let moved = scroll
                    .get("moved")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if resolved && moved {
                    self.page.sleep_ms(250).await?;
                    continue;
                }
            }

            let mut matches = Vec::new();
            for item in &items {
                let raw_labels = self
                    .page
                    .query_all(
                        Some(item),
                        &SelectorQuery::text(display_name, TextMatchMode::Exact),
                        5,
                    )
                    .await?;
                let visible_labels = visible_candidates(self.page, &raw_labels, 5).await?;
                let mut interactive_labels = Vec::new();
                for label in visible_labels {
                    if !self.page.is_decorative(&label).await? {
                        interactive_labels.push(label);
                    }
                }
                if interactive_labels.len() == 1 {
                    matches.push(item.clone());
                }
            }

            diagnostics = json!({
                "candidate_count": total_items,
                "visible_count": items.len(),
                "matching_items": matches.len(),
                "scroll": scroll,
            });

            if matches.len() == 1 {
                let chosen = &matches[0];
                self.page.scroll_into_view(chosen, 5_000).await?;
                let verified = require_target(
                    self.page,
                    std::slice::from_ref(chosen),
                    &ComponentContext::new(
                        Self::NAME,
                        Self::VERSION,
                        "chat.conversation.open",
                        "versioned session items -> left list region -> exact protocol label",
                    ),
                    &TargetOptions::default().require_enabled().scope(&container),
                )
                .await?;
                target = Some(verified);
                break;
            }

            if matches.len() > 1 {
                break;
            }

            scroll = self.scroll_local_viewport(&container, false).await?;
            let resolved = scroll
                .get("resolved")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let moved = scroll
                .get("moved")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !resolved || !moved {
                break;
            }
            self.page.sleep_ms(250).await?;
        }

        let Some(target) = target else {
            let mut details = serde_json::Map::new();
            if let Value::Object(map) = diagnostics {
                for (k, v) in map {
                    details.insert(k, v);
                }
            }
            details.insert(
                "geometry_reason".to_owned(),
                json!("left_list_target_not_unique"),
            );
            return Err(drift_error(
                "The left conversation list did not resolve the protocol-selected target.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "chat.conversation.open",
                    "versioned session items -> left list region -> exact protocol label",
                ),
                Value::Object(details),
            ));
        };

        let class_name = self
            .page
            .get_attribute(&target, "class")
            .await?
            .unwrap_or_default();
        if class_name.contains(Self::SELECTED_CLASS) {
            return Ok(0);
        }

        let click_evidence = click_verified_target(
            self.page,
            &target,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.conversation.open",
                "versioned session items -> exact protocol label -> stable hit target -> dynamic mouse click",
            ),
            ClickOptions::default().without_scroll(),
        )
        .await?;

        let selected_deadline = self.page.now_ms().saturating_add(5_000);
        while self.page.now_ms() < selected_deadline {
            let current_class = self
                .page
                .get_attribute(&target, "class")
                .await?
                .unwrap_or_default();
            if current_class.contains(Self::SELECTED_CLASS) {
                return Ok(1);
            }
            self.page.sleep_ms(100).await?;
        }

        Err(drift_error(
            "The verified conversation click did not select the target row.",
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.conversation.open",
                "versioned session items -> exact protocol label -> stable hit target -> selected-state confirmation",
            ),
            json!({
                "geometry_reason": "conversation_not_selected_after_click",
                "click_evidence": click_evidence,
            }),
        ))
    }

    /// Run a diagnostic probe for the conversation list component.
    pub async fn probe(&self, exercised: bool) -> Result<Value> {
        if !exercised {
            return Ok(not_exercised_probe(
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "chat.conversation.open",
                    "unique conversation list -> local data-e2e items",
                ),
                "chat_route_not_open",
            ));
        }
        let mut evidence = self.evidence(15_000).await?;
        let loaded = evidence
            .get("loaded")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let container_count = evidence
            .get("container_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let left_region_count = evidence
            .get("left_region_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if let Value::Object(ref mut map) = evidence {
            map.insert("capability".to_owned(), json!("chat.conversation.open"));
            map.insert("exercised".to_owned(), json!(true));
            map.insert(
                "status".to_owned(),
                json!(if loaded { "ok" } else { "drifted" }),
            );
            map.insert("visible_count".to_owned(), json!(left_region_count));
            map.insert("unique".to_owned(), json!(container_count == 1));
            map.insert("geometry_valid".to_owned(), json!(container_count == 1));
            map.insert(
                "geometry_reason".to_owned(),
                if loaded {
                    Value::Null
                } else {
                    json!("left_list_not_loaded")
                },
            );
        }
        Ok(evidence)
    }
}

/// Chat message input composer (`chat-composer`, version `2026-08-03.1`).
pub struct ChatComposerComponent<'a> {
    page: &'a dyn UiPage,
}

impl<'a> ChatComposerComponent<'a> {
    pub const NAME: &'static str = "chat-composer";
    pub const VERSION: &'static str = "2026-08-03.1";
    pub const COMPOSER_SELECTOR: &'static str =
        r#"[data-e2e="msg-input"] [contenteditable="true"]"#;
    pub const CONTAINER_XPATH: &'static str =
        "ancestor::div[contains(@class,'messageMsgInputinputRow')][1]";
    pub const SEND_SELECTOR: &'static str = "svg.e2e-send-msg-btn";

    /// Construct a chat composer component bound to `page`.
    pub const fn new(page: &'a dyn UiPage) -> Self {
        Self { page }
    }

    /// Query candidate composer contenteditable nodes.
    pub async fn candidates(&self) -> Result<Vec<ElementId>> {
        self.page
            .query_all(
                None,
                &SelectorQuery::css(Self::COMPOSER_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await
    }

    /// Resolve the unique visible composer editor element.
    pub async fn editor(&self) -> Result<ElementId> {
        let candidates = self.candidates().await?;
        unique_visible(
            self.page,
            &candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.compose",
                "data-e2e msg-input -> visible contenteditable",
            ),
        )
        .await
    }

    /// Resolve the unique local input-row ancestor containing the composer editor.
    pub async fn container(&self) -> Result<ElementId> {
        let editor = self.editor().await?;
        let candidates = self
            .page
            .query_all(
                Some(&editor),
                &SelectorQuery::xpath(Self::CONTAINER_XPATH),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let body = page_body(self.page).await?;
        let mut opts = TargetOptions::default();
        if let Some(ref b) = body {
            opts = opts.scope(b);
        }
        require_target(
            self.page,
            &candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.compose",
                "contenteditable -> unique local input-row ancestor",
            ),
            &opts,
        )
        .await
    }

    /// Resolve the unique `[data-e2e="msg-input"]` composer root.
    pub async fn root(&self) -> Result<ElementId> {
        let candidates = self
            .page
            .query_all(
                None,
                &SelectorQuery::css(r#"[data-e2e="msg-input"]"#),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let body = page_body(self.page).await?;
        let mut opts = TargetOptions::default();
        if let Some(ref b) = body {
            opts = opts.scope(b);
        }
        require_target(
            self.page,
            &candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.compose",
                "unique data-e2e msg-input root",
            ),
            &opts,
        )
        .await
    }

    /// Validate that the composer's send icon is uniquely visible, enabled, and rightmost among local SVGs.
    pub async fn send_control(&self) -> Result<ElementId> {
        let container = self.container().await?;
        let send_candidates = self
            .page
            .query_all(
                Some(&container),
                &SelectorQuery::css(Self::SEND_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let svg_peers = self
            .page
            .query_all(
                Some(&container),
                &SelectorQuery::css("svg"),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        require_target(
            self.page,
            &send_candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.send",
                "composer-local e2e send control -> rightmost local SVG",
            ),
            &TargetOptions::default()
                .require_enabled()
                .scope(&container)
                .peers(&svg_peers, PeerPosition::Rightmost, 2),
        )
        .await
    }

    /// Validate the send control and trigger submission via `Enter` on the focused editor.
    pub async fn send(&self) -> Result<usize> {
        let _ = self.send_control().await?;
        let editor = self.editor().await?;
        self.page.press_element_key(&editor, "Enter", 5_000).await?;
        Ok(1)
    }

    /// Focus the editor via a verified dynamic mouse click, clear its text, and confirm it is empty.
    pub async fn clear(&self) -> Result<ElementId> {
        let editor = self.editor().await?;
        let _ = click_verified_target(
            self.page,
            &editor,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.compose.clear",
                "unique editor -> stable hit target -> dynamic mouse focus",
            ),
            ClickOptions::default().without_scroll(),
        )
        .await?;
        let focused = self
            .page
            .eval_on_element(&editor, ACTIVE_ELEMENT_JS, None)
            .await?
            .as_bool()
            .unwrap_or(false);
        if !focused {
            return Err(drift_error(
                "The verified composer focus action did not focus the editor.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "chat.compose.clear",
                    "unique editor -> stable hit target -> activeElement confirmation",
                ),
                json!({ "geometry_reason": "composer_not_focused_after_action" }),
            ));
        }
        self.page
            .press_element_key(&editor, "Control+A", 5_000)
            .await?;
        self.page
            .press_element_key(&editor, "Backspace", 5_000)
            .await?;
        if !self.text().await?.is_empty() {
            return Err(drift_error(
                "The confirmed chat composer did not clear.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "chat.compose.clear",
                    "unique contenteditable -> semantic fill empty -> state check",
                ),
                json!({ "geometry_reason": "composer_clear_state_mismatch" }),
            ));
        }
        Ok(editor)
    }

    /// Type `text` sequentially into the composer editor with a 35ms per-character delay.
    pub async fn insert_text(&self, text: &str) -> Result<ElementId> {
        let editor = self.editor().await?;
        self.page
            .type_text_sequentially(&editor, text, 35, 30_000)
            .await?;
        Ok(editor)
    }

    /// Read the normalized composer text with CRLF, trailing newlines, and zero-width sentinels stripped.
    pub async fn text(&self) -> Result<String> {
        let editor = self.editor().await?;
        let raw = self.page.inner_text(&editor, 5_000).await?;
        Ok(normalize_rendered_text(&raw))
    }

    /// Run a diagnostic probe for the chat composer editor.
    pub async fn probe(&self, exercised: bool) -> Result<Value> {
        let candidates = if exercised {
            self.candidates().await?
        } else {
            Vec::new()
        };
        component_probe(
            self.page,
            &candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.compose",
                "data-e2e msg-input -> contenteditable",
            ),
            exercised,
        )
        .await
    }

    /// Run diagnostic probes for both composing and sending.
    pub async fn operation_probes(&self, exercised: bool) -> Result<Vec<Value>> {
        let composer = self.probe(exercised).await?;
        let send = resolution_probe(
            self.page,
            || self.send_control(),
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.send",
                "composer -> local input row -> rightmost send control",
            ),
            exercised,
            "conversation_not_selected",
        )
        .await?;
        Ok(vec![composer, send])
    }
}

/// Chat message item and context-menu action component (`chat-message-item`, version `2026-08-03.1`).
pub struct MessageItemComponent<'a> {
    page: &'a dyn UiPage,
}

impl<'a> MessageItemComponent<'a> {
    pub const NAME: &'static str = "chat-message-item";
    pub const VERSION: &'static str = "2026-08-03.1";
    pub const ITEM_SELECTOR: &'static str = r#"[data-e2e="msg-item-content"]"#;
    pub const MENU_READY_CLASS: &'static str = "MessageOperatePopWindowisReady";

    /// Construct a message item component bound to `page`.
    pub const fn new(page: &'a dyn UiPage) -> Self {
        Self { page }
    }

    /// Query candidate message content elements.
    pub async fn candidates(&self) -> Result<Vec<ElementId>> {
        self.page
            .query_all(None, &SelectorQuery::css(Self::ITEM_SELECTOR), 100)
            .await
    }

    /// Run a diagnostic probe for message items.
    pub async fn probe(&self, exercised: bool) -> Result<Value> {
        let candidates = if exercised {
            self.candidates().await?
        } else {
            Vec::new()
        };
        component_probe(
            self.page,
            &candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.message.target",
                "data-e2e message item plus protocol text",
            ),
            exercised,
        )
        .await
    }

    async fn resolve_matching_items(&self, text: &str) -> Result<(Vec<ElementId>, usize)> {
        let candidates = self.candidates().await?;
        let total = candidates.len().min(100);
        let mut exact = Vec::new();
        for item in visible_candidates(self.page, &candidates, 100).await? {
            let value = self
                .page
                .eval_on_element(&item, MESSAGE_EVIDENCE_JS, None)
                .await?;
            let evidence: MessageEvidence = serde_json::from_value(value).unwrap_or_default();
            if message_semantics_match(text, &evidence) {
                exact.push(item);
            }
        }
        Ok((exact, total))
    }

    /// Resolve a unique visible message item matching protocol `text` (including inline emoji alt/title).
    pub async fn target(&self, text: &str, capability: &str) -> Result<ElementId> {
        let body_candidates = self
            .page
            .query_all(None, &SelectorQuery::css("body"), 1)
            .await?;
        let root = require_target(
            self.page,
            &body_candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                capability,
                "page body local scope",
            ),
            &TargetOptions::default(),
        )
        .await?;

        let (exact, total) = self.resolve_matching_items(text).await?;
        if exact.len() != 1 {
            return Err(drift_error(
                "The protocol-selected message component is not unique.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    capability,
                    "data-e2e message item -> exact rendered text plus local emoji semantics",
                ),
                json!({
                    "candidate_count": exact.len(),
                    "visible_message_items": total,
                }),
            ));
        }

        self.page.scroll_into_view(&exact[0], 10_000).await?;
        self.page.sleep_ms(150).await?;

        let (exact, total) = self.resolve_matching_items(text).await?;
        if exact.len() != 1 {
            return Err(drift_error(
                "The virtualized message component did not remain uniquely bound after scrolling.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    capability,
                    "protocol text plus emoji semantics -> component scroll -> post-scroll semantic rebind",
                ),
                json!({
                    "candidate_count": exact.len(),
                    "visible_message_items": total,
                    "geometry_reason": "message_rebind_not_unique_after_scroll",
                }),
            ));
        }

        require_target(
            self.page,
            &exact,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                capability,
                "data-e2e message item -> exact rendered text plus local emoji semantics",
            ),
            &TargetOptions::default().scope(&root),
        )
        .await
    }

    /// Resolve the local context menu popup anchored near `item`.
    pub async fn context_menu(&self, item: &ElementId, capability: &str) -> Result<ElementId> {
        let candidates = CHAT_ADAPTER
            .query_selector(self.page, "message_context_menu", DEFAULT_CANDIDATE_LIMIT)
            .await?;
        require_target(
            self.page,
            &candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                capability,
                "target message -> unique versioned local action popup",
            ),
            &TargetOptions::default()
                .anchor(item, None, Some(700.0))
                .filter_by_geometry(),
        )
        .await
    }

    /// Open the right-click context menu on `item`, wait for readiness and geometry stability, and click `label`.
    pub async fn menu_action(
        &self,
        item: &ElementId,
        label: &str,
        capability: &str,
    ) -> Result<usize> {
        self.page
            .click_element(item, MouseButton::Right, 15_000)
            .await?;

        // Native CDP right-click finishes in <1ms; allow the popup container a brief bounded
        // window to mount and become visible before declaring drift.
        let mount_deadline = self.page.now_ms().saturating_add(2_000);
        let menu = loop {
            match self.context_menu(item, capability).await {
                Ok(found) => break found,
                Err(err) => {
                    if self.page.now_ms() >= mount_deadline {
                        return Err(err);
                    }
                    self.page.sleep_ms(50).await?;
                }
            }
        };

        let ready_deadline = self.page.now_ms().saturating_add(3_000);
        let mut ready = false;
        while self.page.now_ms() < ready_deadline {
            let class_name = self
                .page
                .get_attribute(&menu, "class")
                .await?
                .unwrap_or_default();
            if class_name.contains(Self::MENU_READY_CLASS) {
                ready = true;
                break;
            }
            self.page.sleep_ms(50).await?;
        }
        if !ready {
            let _ = self.page.press_key("Escape", 2_000).await;
            let _ = self.page.click_xy(4.0, 4.0, MouseButton::Left, 2_000).await;
            return Err(drift_error(
                "The message action menu did not reach its ready state.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    capability,
                    "target message -> local menu -> ready state",
                ),
                json!({ "geometry_reason": "message_menu_not_ready" }),
            ));
        }

        let action_selector = CHAT_ADAPTER
            .selector("message_context_action")
            .unwrap_or(".MessageOperatePopBodybuttonItem");
        let action_rows = self
            .page
            .query_all(
                Some(&menu),
                &SelectorQuery::css(action_selector),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let mut exact_rows = Vec::new();
        let mut available_actions = Vec::new();
        for row in visible_candidates(self.page, &action_rows, DEFAULT_CANDIDATE_LIMIT).await? {
            let text = self.page.inner_text(&row, 5_000).await?;
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                available_actions.push(trimmed.to_owned());
            }
            if trimmed == label {
                exact_rows.push(row);
            }
        }
        if exact_rows.len() != 1 {
            let _ = self.page.press_key("Escape", 2_000).await;
            let _ = self.page.click_xy(4.0, 4.0, MouseButton::Left, 2_000).await;
            return Err(drift_error(
                "The message action menu did not resolve one exact action row.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    capability,
                    "target message -> ready local action popup -> exact semantic action row",
                ),
                json!({
                    "candidate_count": exact_rows.len(),
                    "available_actions": available_actions,
                    "geometry_reason": "message_menu_action_not_unique",
                }),
            ));
        }

        let action = require_target(
            self.page,
            &exact_rows,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                capability,
                "target message -> ready local action popup -> exact semantic action row",
            ),
            &TargetOptions::default()
                .require_enabled()
                .scope(&menu)
                .anchor(item, None, Some(600.0))
                .filter_by_geometry(),
        )
        .await?;

        let mut stable_samples = 0usize;
        let mut previous_box = None;
        let stable_deadline = self.page.now_ms().saturating_add(3_000);
        while self.page.now_ms() < stable_deadline {
            let current_box = self.page.bounding_box(&action).await?;
            if let (Some(curr), Some(prev)) = (current_box, previous_box) {
                let delta = curr.max_delta(&prev);
                stable_samples = if delta <= 1.0 { stable_samples + 1 } else { 0 };
                if stable_samples >= 3 {
                    break;
                }
            }
            previous_box = current_box;
            self.page.sleep_ms(80).await?;
        }
        if stable_samples < 3 {
            return Err(drift_error(
                "The message action row did not settle before the click.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    capability,
                    "ready local action row -> stable geometry samples",
                ),
                json!({ "geometry_reason": "message_menu_action_unstable" }),
            ));
        }

        let _ = click_verified_target(
            self.page,
            &action,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                capability,
                "ready local action row -> stable hit target -> dynamic mouse click",
            ),
            ClickOptions::default().without_scroll(),
        )
        .await
        .map_err(|err| {
            if capability == "chat.message.like" && err.code != "PAGE_ADAPTER_DRIFTED" {
                err.mark_write_started()
            } else {
                err
            }
        })?;
        Ok(2)
    }

    /// Open the right-click context menu on `item` and trigger the message like reaction.
    ///
    /// Supports both the classic `"点赞"` text action row and the quick-emoji reaction bar
    /// (`.PopBodyEmojiQuickReplytoggleButton` -> `img[title="[爱心]"]`).
    pub async fn like_action(&self, item: &ElementId, capability: &str) -> Result<usize> {
        self.page
            .click_element(item, MouseButton::Right, 15_000)
            .await?;

        let mount_deadline = self.page.now_ms().saturating_add(2_000);
        let menu = loop {
            match self.context_menu(item, capability).await {
                Ok(found) => break found,
                Err(err) => {
                    if self.page.now_ms() >= mount_deadline {
                        return Err(err);
                    }
                    self.page.sleep_ms(50).await?;
                }
            }
        };

        let ready_deadline = self.page.now_ms().saturating_add(3_000);
        let mut ready = false;
        while self.page.now_ms() < ready_deadline {
            let class_name = self
                .page
                .get_attribute(&menu, "class")
                .await?
                .unwrap_or_default();
            if class_name.contains(Self::MENU_READY_CLASS) {
                ready = true;
                break;
            }
            self.page.sleep_ms(50).await?;
        }
        if !ready {
            let _ = self.page.press_key("Escape", 2_000).await;
            let _ = self.page.click_xy(4.0, 4.0, MouseButton::Left, 2_000).await;
            return Err(drift_error(
                "The message action menu did not reach its ready state.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    capability,
                    "target message -> local menu -> ready state",
                ),
                json!({ "geometry_reason": "message_menu_not_ready" }),
            ));
        }

        // Path 1: Classic "点赞" text action row.
        let like_label = CHAT_ADAPTER.label("like").unwrap_or("点赞");
        let action_selector = CHAT_ADAPTER
            .selector("message_context_action")
            .unwrap_or(".MessageOperatePopBodybuttonItem");
        let action_rows = self
            .page
            .query_all(
                Some(&menu),
                &SelectorQuery::css(action_selector),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let mut exact_rows = Vec::new();
        let mut available_actions = Vec::new();
        for row in visible_candidates(self.page, &action_rows, DEFAULT_CANDIDATE_LIMIT).await? {
            let text = self.page.inner_text(&row, 5_000).await?;
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                available_actions.push(trimmed.to_owned());
            }
            if trimmed == like_label {
                exact_rows.push(row);
            }
        }
        if exact_rows.len() == 1 {
            let action = require_target(
                self.page,
                &exact_rows,
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    capability,
                    "target message -> ready local action popup -> exact semantic action row",
                ),
                &TargetOptions::default()
                    .require_enabled()
                    .scope(&menu)
                    .anchor(item, None, Some(600.0))
                    .filter_by_geometry(),
            )
            .await?;
            let _ = click_verified_target(
                self.page,
                &action,
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    capability,
                    "ready local action row -> stable hit target -> dynamic mouse click",
                ),
                ClickOptions::default().without_scroll(),
            )
            .await
            .map_err(|err| {
                if err.code == "PAGE_ADAPTER_DRIFTED" {
                    err
                } else {
                    err.mark_write_started()
                }
            })?;
            return Ok(2);
        }

        // Path 2: Quick-emoji reaction bar (`.PopBodyEmojiQuickReplytoggleButton` -> `img[title="[爱心]"]`).
        let emoji_like_selector = CHAT_ADAPTER
            .selector("message_emoji_like")
            .unwrap_or(r#"img[title="[爱心]"]"#);
        let mut heart_candidates = self
            .page
            .query_all(
                Some(&menu),
                &SelectorQuery::css(emoji_like_selector),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let mut visible_hearts =
            visible_candidates(self.page, &heart_candidates, DEFAULT_CANDIDATE_LIMIT).await?;
        let mut dom_ops = 1usize;

        if visible_hearts.is_empty() {
            let toggle_selector = CHAT_ADAPTER
                .selector("message_emoji_toggle")
                .unwrap_or(".PopBodyEmojiQuickReplytoggleButton");
            let toggles = self
                .page
                .query_all(
                    Some(&menu),
                    &SelectorQuery::css(toggle_selector),
                    DEFAULT_CANDIDATE_LIMIT,
                )
                .await?;
            let visible_toggles =
                visible_candidates(self.page, &toggles, DEFAULT_CANDIDATE_LIMIT).await?;
            if visible_toggles.len() == 1 {
                self.page
                    .click_element(&visible_toggles[0], MouseButton::Left, 5_000)
                    .await?;
                dom_ops += 1;
                self.page.sleep_ms(350).await?;
                heart_candidates = self
                    .page
                    .query_all(
                        Some(&menu),
                        &SelectorQuery::css(emoji_like_selector),
                        DEFAULT_CANDIDATE_LIMIT,
                    )
                    .await?;
                if let Some(first) = heart_candidates.first() {
                    let _ = self.page.scroll_into_view(first, 5_000).await;
                    self.page.sleep_ms(120).await?;
                }
                visible_hearts =
                    visible_candidates(self.page, &heart_candidates, DEFAULT_CANDIDATE_LIMIT)
                        .await?;
            }
        }

        if visible_hearts.len() == 1 {
            self.page
                .click_element(&visible_hearts[0], MouseButton::Left, 10_000)
                .await
                .map_err(DoudouError::mark_write_started)?;
            return Ok(dom_ops + 1);
        }

        let _ = self.page.press_key("Escape", 2_000).await;
        let _ = self.page.click_xy(4.0, 4.0, MouseButton::Left, 2_000).await;
        Err(drift_error(
            "The message action menu did not resolve one exact action row.",
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                capability,
                "target message -> ready local action popup -> exact semantic action row or reaction heart",
            ),
            json!({
                "candidate_count": exact_rows.len(),
                "reaction_heart_count": visible_hearts.len(),
                "available_actions": available_actions,
                "geometry_reason": "message_menu_action_not_unique",
            }),
        ))
    }

    /// Return a `not_exercised` diagnostic probe entry for the message context menu.
    pub async fn context_menu_probe(&self) -> Result<Value> {
        let candidates = CHAT_ADAPTER
            .query_selector(self.page, "message_context_menu", DEFAULT_CANDIDATE_LIMIT)
            .await?;
        let visible = visible_candidates(self.page, &candidates, DEFAULT_CANDIDATE_LIMIT).await?;
        let reason = if visible.is_empty() {
            "context_menu_not_open"
        } else {
            "target_message_not_bound"
        };
        Ok(not_exercised_probe(
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.message.context_menu",
                "target message -> versioned local action popup",
            ),
            reason,
        ))
    }
}

/// Chat image upload and preview modal component (`chat-image-preview`, version `2026-08-03.1`).
pub struct ImagePreviewComponent<'a> {
    page: &'a dyn UiPage,
    source: Option<PathBuf>,
}

impl<'a> ImagePreviewComponent<'a> {
    pub const NAME: &'static str = "chat-image-preview";
    pub const VERSION: &'static str = "2026-08-03.1";
    pub const UPLOAD_SELECTOR: &'static str = r#"[role="button"].semi-upload-add"#;

    /// Construct an image preview component bound to `page`.
    pub const fn new(page: &'a dyn UiPage) -> Self {
        Self { page, source: None }
    }

    /// Query composer-local image upload button candidates.
    pub async fn candidates(&self) -> Result<Vec<ElementId>> {
        let roots = self
            .page
            .query_all(
                None,
                &SelectorQuery::css(r#"[data-e2e="msg-input"]"#),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let mut results = Vec::new();
        for root in roots {
            let found = self
                .page
                .query_all(
                    Some(&root),
                    &SelectorQuery::css(Self::UPLOAD_SELECTOR),
                    DEFAULT_CANDIDATE_LIMIT,
                )
                .await?;
            results.extend(found);
        }
        Ok(results)
    }

    /// Run a diagnostic probe for the upload trigger button.
    pub async fn probe(&self, exercised: bool) -> Result<Value> {
        let candidates = if exercised {
            self.candidates().await?
        } else {
            Vec::new()
        };
        component_probe(
            self.page,
            &candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.image.choose",
                "composer-local upload role button",
            ),
            exercised,
        )
        .await
    }

    /// Trigger the composer-local upload control and bind `source` to the file chooser.
    pub async fn choose(&mut self, source: &Path) -> Result<usize> {
        let roots = self
            .page
            .query_all(
                None,
                &SelectorQuery::css(r#"[data-e2e="msg-input"]"#),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let composer_root = require_target(
            self.page,
            &roots,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.image.choose",
                "unique visible composer root",
            ),
            &TargetOptions::default(),
        )
        .await?;

        let uploads = self
            .page
            .query_all(
                Some(&composer_root),
                &SelectorQuery::css(Self::UPLOAD_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let upload = unique_visible(
            self.page,
            &uploads,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.image.choose",
                "composer-local upload role button",
            ),
        )
        .await?;
        let upload = require_target(
            self.page,
            &[upload],
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.image.choose",
                "composer-local upload role button",
            ),
            &TargetOptions::default()
                .require_enabled()
                .scope(&composer_root),
        )
        .await?;

        self.page
            .choose_files_via_click(&upload, &[source], 10_000)
            .await?;
        self.source = Some(source.to_path_buf());
        Ok(2)
    }

    /// Resolve the unique visible image preview modal dialog.
    pub async fn preview(&self) -> Result<ElementId> {
        let candidates = CHAT_ADAPTER
            .query_selector(self.page, "image_preview_dialog", DEFAULT_CANDIDATE_LIMIT)
            .await?;
        require_target(
            self.page,
            &candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.image.send",
                "uploaded image -> unique visible preview/dialog",
            ),
            &TargetOptions::default(),
        )
        .await
    }

    /// Verify the preview image, exact source filename label, and send button, then click send.
    pub async fn send(&self) -> Result<usize> {
        let preview = self.preview().await?;
        let images = self
            .page
            .query_all(
                Some(&preview),
                &SelectorQuery::css("img"),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let image = require_target(
            self.page,
            &images,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.image.send",
                "preview dialog -> unique visible image",
            ),
            &TargetOptions::default().scope(&preview),
        )
        .await?;

        let Some(ref source) = self.source else {
            return Err(drift_error(
                "The image preview is not bound to a selected source file.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "chat.image.send",
                    "selected source -> preview dialog -> exact file label",
                ),
                json!({ "geometry_reason": "image_source_not_bound" }),
            ));
        };
        let file_name = source.file_name().and_then(|s| s.to_str()).ok_or_else(|| {
            DoudouError::new(
                "INVALID_IMAGE_PATH",
                "Image path has no valid UTF-8 filename.",
            )
        })?;
        let labels = self
            .page
            .query_all(
                Some(&preview),
                &SelectorQuery::text(file_name, TextMatchMode::Exact),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let _ = require_target(
            self.page,
            &labels,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.image.send",
                "selected source -> preview dialog -> exact file label",
            ),
            &TargetOptions::default().scope(&preview),
        )
        .await?;

        let send_selector = CHAT_ADAPTER
            .selector("image_preview_send")
            .unwrap_or("button.MsgInputSendFileModalbtnSure");
        let send_buttons = self
            .page
            .query_all(
                Some(&preview),
                &SelectorQuery::css(send_selector),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let action = require_target(
            self.page,
            &send_buttons,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.image.send",
                "preview dialog -> exact source label -> versioned send action",
            ),
            &TargetOptions::default()
                .require_enabled()
                .scope(&preview)
                .anchor(
                    &image,
                    Some(crate::geometry::AnchorRelation::Below),
                    Some(1000.0),
                ),
        )
        .await?;

        self.page
            .click_element(&action, MouseButton::Left, 10_000)
            .await
            .map_err(DoudouError::mark_write_started)?;
        Ok(1)
    }

    /// Run a diagnostic probe for the image preview dialog.
    pub async fn preview_probe(&self) -> Result<Value> {
        let candidates = CHAT_ADAPTER
            .query_selector(self.page, "image_preview_dialog", DEFAULT_CANDIDATE_LIMIT)
            .await?;
        let visible = !visible_candidates(self.page, &candidates, DEFAULT_CANDIDATE_LIMIT)
            .await?
            .is_empty();
        resolution_probe(
            self.page,
            || self.preview(),
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "chat.image.send",
                "uploaded image -> unique preview dialog -> local action",
            ),
            visible,
            "image_preview_not_open",
        )
        .await
    }
}
