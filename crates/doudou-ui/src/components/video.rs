//! Douyin video, profile list, tutorial dialog, comment panel, composer, and comment item components.

use std::sync::LazyLock;

use doudou_core::{DoudouError, Result};
use regex::Regex;
use serde_json::{Value, json};

use crate::adapters::{PROFILE_ADAPTER, VIDEO_ADAPTER};
use crate::geometry::{
    ClickOptions, ComponentContext, DEFAULT_CANDIDATE_LIMIT, PeerPosition, TargetOptions,
    click_verified_target, component_probe, drift_error, not_exercised_probe, require_target,
    resolution_probe, visible_candidates,
};
use crate::page::{ElementId, MouseButton, SelectorQuery, TextMatchMode, UiPage, UiSession};

static EMOJI_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[[^\[\]\r\n]{1,24}\]").expect("valid emoji token regex"));

static REPLY_EXPAND_LABEL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(展开|查看).*回复").expect("valid reply expand label regex"));

const ACTIVE_ELEMENT_JS: &str =
    "element => element === document.activeElement || element.contains(document.activeElement)";

const COMPOSED_TEXT_JS: &str = "element => 'value' in element ? element.value : (element.innerText || element.textContent || '')";

const PROFILE_SCROLL_TO_END_JS: &str = "element => { const before=element.scrollTop; element.scrollTop=element.scrollHeight; return element.scrollTop !== before; }";

const COMMENT_SCROLL_TOWARD_END_JS: &str = r"element => {
  const scrollable = node => {
    const style = getComputedStyle(node);
    return node.scrollHeight > node.clientHeight + 20
      && ['auto', 'scroll'].includes(style.overflowY);
  };
  const preferredNodes = [
    ...(element.matches('.comment-mainContent') ? [element] : []),
    ...element.querySelectorAll('.comment-mainContent')
  ];
  for (let node = element.parentElement; node; node = node.parentElement) {
    if (node.matches('.comment-mainContent')) preferredNodes.push(node);
  }
  const preferred = preferredNodes.filter(scrollable);
  if (preferredNodes.length && preferred.length !== 1) {
    return {
      resolved: false,
      reason: 'comment_main_content_not_unique_or_scrollable',
      preferred_count: preferredNodes.length,
      preferred_scrollable_count: preferred.length,
    };
  }
  const local = preferred.length ? preferred : [element, ...element.querySelectorAll('div')]
    .filter(scrollable);
  const ancestors = [];
  for (let node = element.parentElement; node; node = node.parentElement) {
    if (scrollable(node)) ancestors.push(node);
  }
  const candidates = local.length ? local : ancestors;
  if (candidates.length !== 1) {
    return {
      resolved: false,
      reason: 'comment_scroll_root_not_unique',
      local_count: local.length,
      ancestor_count: ancestors.length,
    };
  }
  const scroller = candidates[0];
  const before = scroller.scrollTop;
  scroller.scrollTop = Math.min(
    scroller.scrollHeight,
    before + Math.max(500, scroller.clientHeight * 0.8)
  );
  return {
    resolved: true,
    source: preferred.length ? 'comment-mainContent' : (local.length ? 'local' : 'ancestor'),
    before,
    after: scroller.scrollTop,
    moved: scroller.scrollTop !== before,
    client_height: scroller.clientHeight,
    scroll_height: scroller.scrollHeight,
  };
}";

/// Strip inline bracketed emoji tokens (`[微笑]`) from protocol comment text to match visible DOM text.
pub fn visible_comment_text(protocol_text: &str) -> String {
    EMOJI_TOKEN_RE
        .replace_all(protocol_text, "")
        .trim()
        .to_owned()
}

async fn page_body(page: &dyn UiPage) -> Result<Option<ElementId>> {
    let nodes = page.query_all(None, &SelectorQuery::css("body"), 1).await?;
    Ok(nodes.into_iter().next())
}

/// First-run video tutorial dialog dismissal component (`video-tutorial-dialog`).
#[derive(Debug, Default, Clone, Copy)]
pub struct TutorialDialogComponent;

impl TutorialDialogComponent {
    pub const NAME: &'static str = "video-tutorial-dialog";
    pub const VERSION: &'static str = "2026-08-02.1";
    pub const DIALOG_XPATH: &'static str =
        "ancestor::*[@role='dialog' or contains(@class,'modal') or contains(@class,'guide')][1]";

    /// Dismiss the tutorial dialog if present, returning `1` when dismissed or `0` when absent.
    pub async fn dismiss(&self, page: &dyn UiPage) -> Result<usize> {
        let label = VIDEO_ADAPTER.label("tutorial").unwrap_or("我知道了");
        let raw = page
            .query_all(None, &SelectorQuery::text(label, TextMatchMode::Exact), 10)
            .await?;
        let visible = visible_candidates(page, &raw, 10).await?;
        if visible.is_empty() {
            return Ok(0);
        }
        let action = self.action(page).await?;
        page.click_element(&action, MouseButton::Left, 10_000)
            .await?;
        Ok(1)
    }

    /// Resolve the unique tutorial dismiss button scoped to its modal/guide dialog.
    pub async fn action(&self, page: &dyn UiPage) -> Result<ElementId> {
        let label = VIDEO_ADAPTER.label("tutorial").unwrap_or("我知道了");
        let labels = page
            .query_all(
                None,
                &SelectorQuery::text(label, TextMatchMode::Exact),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let action = require_target(
            page,
            &labels,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.tutorial.dismiss",
                "stable tutorial action label",
            ),
            &TargetOptions::default().require_enabled(),
        )
        .await?;

        let dialogs = page
            .query_all(
                Some(&action),
                &SelectorQuery::xpath(Self::DIALOG_XPATH),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let body = page_body(page).await?;
        let mut dialog_opts = TargetOptions::default();
        if let Some(ref b) = body {
            dialog_opts = dialog_opts.scope(b);
        }
        let dialog = require_target(
            page,
            &dialogs,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.tutorial.dismiss",
                "tutorial action -> unique local dialog",
            ),
            &dialog_opts,
        )
        .await?;

        let scoped_labels = page
            .query_all(
                Some(&dialog),
                &SelectorQuery::text(label, TextMatchMode::Exact),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        require_target(
            page,
            &scoped_labels,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.tutorial.dismiss",
                "tutorial dialog -> local semantic action",
            ),
            &TargetOptions::default().require_enabled().scope(&dialog),
        )
        .await
    }

    /// Probe the tutorial dialog component if present.
    pub async fn probe(&self, page: &dyn UiPage) -> Result<Value> {
        let label = VIDEO_ADAPTER.label("tutorial").unwrap_or("我知道了");
        let raw = page
            .query_all(None, &SelectorQuery::text(label, TextMatchMode::Exact), 10)
            .await?;
        let visible = !visible_candidates(page, &raw, 10).await?.is_empty();
        resolution_probe(
            page,
            || self.action(page),
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.tutorial.dismiss",
                "semantic tutorial action -> local dialog -> local action",
            ),
            visible,
            "tutorial_dialog_not_present",
        )
        .await
    }
}

/// User profile video list pagination component (`profile-video-list`).
pub struct ProfileVideoListComponent<'a> {
    page: &'a dyn UiPage,
}

impl<'a> ProfileVideoListComponent<'a> {
    pub const NAME: &'static str = "profile-video-list";
    pub const VERSION: &'static str = "2026-08-02.1";

    /// Construct a profile video list component bound to `page`.
    pub const fn new(page: &'a dyn UiPage) -> Self {
        Self { page }
    }

    /// Resolve the unique profile scroll container.
    pub async fn scroll_container(&self) -> Result<ElementId> {
        let candidates = PROFILE_ADAPTER
            .query_selector(self.page, "scroll_container", DEFAULT_CANDIDATE_LIMIT)
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
                "video.list.paginate",
                "profile route -> versioned scroll container",
            ),
            &opts,
        )
        .await
    }

    /// Scroll the profile video list to the bottom, returning `true` if `scrollTop` changed.
    pub async fn scroll_to_end(&self) -> Result<bool> {
        let container = self.scroll_container().await?;
        let moved = self
            .page
            .eval_on_element(&container, PROFILE_SCROLL_TO_END_JS, None)
            .await?
            .as_bool()
            .unwrap_or(false);
        Ok(moved)
    }
}

/// Video detail `#RENDER_DATA` reader (`video-detail`).
pub struct VideoDetailComponent<'a> {
    page: &'a dyn UiPage,
}

impl<'a> VideoDetailComponent<'a> {
    pub const NAME: &'static str = "video-detail";
    pub const VERSION: &'static str = "2026-08-02.1";

    /// Construct a video detail component bound to `page`.
    pub const fn new(page: &'a dyn UiPage) -> Self {
        Self { page }
    }

    /// Read the unique `#RENDER_DATA` text payload, or `None` when absent.
    pub async fn render_data(&self) -> Result<Option<String>> {
        let candidates = VIDEO_ADAPTER
            .query_selector(self.page, "render_data", DEFAULT_CANDIDATE_LIMIT)
            .await?;
        if candidates.is_empty() {
            return Ok(None);
        }
        if candidates.len() != 1 {
            return Err(drift_error(
                "The structured video detail node is not unique.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "video.detail.read",
                    "video route -> unique structured render node",
                ),
                json!({ "candidate_count": candidates.len() }),
            ));
        }
        let text = self.page.text_content(&candidates[0], 3_000).await?;
        Ok(Some(text.unwrap_or_default()))
    }
}

/// Video comments drawer/panel component (`video-comments-panel`, version `2026-08-03.1`).
#[derive(Debug, Default, Clone, Copy)]
pub struct VideoCommentsPanelComponent;

impl VideoCommentsPanelComponent {
    pub const NAME: &'static str = "video-comments-panel";
    pub const VERSION: &'static str = "2026-08-03.1";
    pub const COMMENT_BUTTON_SELECTOR: &'static str = r#"[data-e2e="feed-comment-icon"], [data-e2e*="comment" i], button[aria-label*="评论"], [title*="评论"]"#;
    pub const REPLY_EXPAND_SELECTOR: &'static str = "button.comment-reply-expand-btn";
    pub const PANEL_XPATH: &'static str =
        "ancestor::*[.//*[contains(@class,'comment-input-inner-container')]][1]";
    pub const ACTION_BAR_XPATH: &'static str =
        "ancestor::*[@role='toolbar' or contains(@class,'action') or contains(@class,'right')][1]";

    /// Query candidate comment action bar buttons.
    pub async fn candidates(&self, page: &dyn UiPage) -> Result<Vec<ElementId>> {
        page.query_all(
            None,
            &SelectorQuery::css(Self::COMMENT_BUTTON_SELECTOR),
            DEFAULT_CANDIDATE_LIMIT,
        )
        .await
    }

    async fn resolve_panel(&self, page: &dyn UiPage, required: bool) -> Result<Option<ElementId>> {
        let lists = page
            .query_all(
                None,
                &SelectorQuery::css(r#"[data-e2e="comment-list"]"#),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let visible = visible_candidates(page, &lists, DEFAULT_CANDIDATE_LIMIT).await?;
        if visible.is_empty() && !required {
            return Ok(None);
        }
        let comment_list = require_target(
            page,
            &lists,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comments.panel",
                "versioned comment list -> visible local panel",
            ),
            &TargetOptions::default(),
        )
        .await?;

        let panel_candidates = page
            .query_all(
                Some(&comment_list),
                &SelectorQuery::xpath(Self::PANEL_XPATH),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        if panel_candidates.is_empty() {
            return Ok(Some(comment_list));
        }

        let resolved = require_target(
            page,
            &panel_candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comments.panel",
                "comment list -> local panel containing composer",
            ),
            &TargetOptions::default(),
        )
        .await?;
        let _ = require_target(
            page,
            &[comment_list],
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comments.panel",
                "resolved panel -> contained comment list",
            ),
            &TargetOptions::default().scope(&resolved),
        )
        .await?;
        let composer_containers = VIDEO_ADAPTER
            .query_selector(page, "comment_container", DEFAULT_CANDIDATE_LIMIT)
            .await?;
        let _ = require_target(
            page,
            &composer_containers,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comments.panel",
                "resolved panel -> contained comment composer",
            ),
            &TargetOptions::default().scope(&resolved),
        )
        .await?;
        Ok(Some(resolved))
    }

    async fn action_bar_and_button(&self, page: &dyn UiPage) -> Result<(ElementId, ElementId)> {
        let deadline = page.now_ms().saturating_add(20_000);
        let mut button: Option<ElementId> = None;
        while page.now_ms() < deadline {
            let candidates = self.candidates(page).await?;
            let visible = visible_candidates(page, &candidates, DEFAULT_CANDIDATE_LIMIT).await?;
            if visible.len() == 1 {
                let resolved = require_target(
                    page,
                    &visible,
                    &ComponentContext::new(
                        Self::NAME,
                        Self::VERSION,
                        "video.comments.open",
                        "bounded render wait -> unique semantic comment action",
                    ),
                    &TargetOptions::default().require_enabled(),
                )
                .await?;
                button = Some(resolved);
                break;
            }
            if visible.len() > 1 {
                let _ = require_target(
                    page,
                    &candidates,
                    &ComponentContext::new(
                        Self::NAME,
                        Self::VERSION,
                        "video.comments.open",
                        "bounded render wait -> unique semantic comment action",
                    ),
                    &TargetOptions::default().require_enabled(),
                )
                .await?;
            }
            page.sleep_ms(250).await?;
        }

        let button = if let Some(b) = button {
            b
        } else {
            let candidates = self.candidates(page).await?;
            require_target(
                page,
                &candidates,
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "video.comments.open",
                    "bounded render wait -> unique semantic comment action",
                ),
                &TargetOptions::default().require_enabled(),
            )
            .await?
        };

        let bar_candidates = page
            .query_all(
                Some(&button),
                &SelectorQuery::xpath(Self::ACTION_BAR_XPATH),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let body = page_body(page).await?;
        let mut bar_opts = TargetOptions::default();
        if let Some(ref b) = body {
            bar_opts = bar_opts.scope(b);
        }
        let bar = require_target(
            page,
            &bar_candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comments.open",
                "comment action -> local video action bar",
            ),
            &bar_opts,
        )
        .await?;

        let local_buttons = page
            .query_all(
                Some(&bar),
                &SelectorQuery::css(Self::COMMENT_BUTTON_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let local = require_target(
            page,
            &local_buttons,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comments.open",
                "video action bar -> local comment action",
            ),
            &TargetOptions::default().require_enabled().scope(&bar),
        )
        .await?;

        Ok((bar, local))
    }

    /// Run a diagnostic probe for the video comments panel or its action bar button.
    pub async fn probe(&self, page: &dyn UiPage, exercised: bool) -> Result<Value> {
        if !exercised {
            return Ok(not_exercised_probe(
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "video.comments.open",
                    "video action bar -> local comment action",
                ),
                "video_route_not_open",
            ));
        }
        let panel_open = self.resolve_panel(page, false).await?.is_some();
        resolution_probe(
            page,
            || async {
                if panel_open {
                    self.resolve_panel(page, true).await?.ok_or_else(|| {
                        DoudouError::new(
                            "PAGE_ADAPTER_DRIFTED",
                            "The comment panel disappeared during probe.",
                        )
                    })
                } else {
                    let (_bar, button) = self.action_bar_and_button(page).await?;
                    Ok(button)
                }
            },
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comments.open",
                "video route -> local action bar/comment panel -> geometry",
            ),
            true,
            "video_route_not_open",
        )
        .await
    }

    /// Open a video URL on `session`, verify the route matches `aweme_id`, dismiss tutorial, and open comments.
    pub async fn open(&self, session: &dyn UiSession, aweme_id: &str) -> Result<usize> {
        session.ensure_ready().await.map_err(|error| {
            if error.code == "INTERNAL_ERROR" {
                drift_error(
                    "The video page did not become ready.",
                    &ComponentContext::new(
                        Self::NAME,
                        Self::VERSION,
                        "video.comments.open",
                        "ensure_ready",
                    ),
                    json!({ "stage": "ensure_ready", "cause_type": error.code }),
                )
            } else {
                error
            }
        })?;
        session
            .open(&format!("https://www.douyin.com/video/{aweme_id}"))
            .await?;

        let page = session.page();
        let current_url = page.url().await?;
        let path = current_url
            .split('?')
            .next()
            .unwrap_or(&current_url)
            .trim_end_matches('/');
        let route_ok = path.ends_with(&format!("/{aweme_id}"))
            && (path.contains("/video/") || path.contains("/note/"));
        if !route_ok {
            return Err(DoudouError::new(
                "VIDEO_DETAIL_MISMATCH",
                "The page content does not match the confirmed plan.",
            )
            .with_details(json!({
                "component": Self::NAME,
                "component_version": Self::VERSION,
                "capability": "video.comments.open",
            })));
        }

        page.sleep_ms(3_000).await?;
        let mut operations = 0usize;
        operations += TutorialDialogComponent.dismiss(page).await?;
        operations += self.ensure_open(page).await?;
        Ok(operations)
    }

    /// Ensure the comments drawer is open on `page`, returning `0` if already open or `1` if opened.
    pub async fn ensure_open(&self, page: &dyn UiPage) -> Result<usize> {
        if self.resolve_panel(page, false).await?.is_some() {
            return Ok(0);
        }
        let (_bar, button) = self.action_bar_and_button(page).await?;
        let open_result: Result<()> = async {
            page.click_element(&button, MouseButton::Left, 15_000)
                .await?;
            for _ in 0..200 {
                if self.resolve_panel(page, false).await?.is_some() {
                    return Ok(());
                }
                page.sleep_ms(100).await?;
            }
            Err(DoudouError::new(
                "TIMEOUT",
                "no visible comment panel component",
            ))
        }
        .await;

        if let Err(error) = open_result {
            return Err(DoudouError::new(
                "PAGE_ADAPTER_DRIFTED",
                "The video comments panel did not become ready.",
            )
            .with_details(json!({
                "component": Self::NAME,
                "component_version": Self::VERSION,
                "capability": "video.comments.open",
                "stage": "open_comment_panel",
                "cause_type": error.code,
            })));
        }
        Ok(1)
    }

    /// Click the topmost reply-expansion button inside the comment panel with bounded retries.
    pub async fn trigger_reply_template(
        &self,
        page: &dyn UiPage,
        max_attempts: usize,
    ) -> Result<usize> {
        let Some(panel) = self.resolve_panel(page, true).await? else {
            return Ok(0);
        };
        let raw_buttons = page
            .query_all(
                Some(&panel),
                &SelectorQuery::css(Self::REPLY_EXPAND_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let visible = visible_candidates(page, &raw_buttons, DEFAULT_CANDIDATE_LIMIT).await?;
        if visible.is_empty() {
            return Ok(0);
        }

        let mut positioned = Vec::new();
        for item in &visible {
            if let Some(b) = page.bounding_box(item).await? {
                positioned.push((item.clone(), b));
            }
        }
        let top_y = positioned
            .iter()
            .map(|(_, b)| b.y)
            .fold(None, |acc: Option<f64>, y| {
                Some(acc.map_or(y, |curr| curr.min(y)))
            });
        let topmost: Vec<ElementId> = positioned
            .into_iter()
            .filter_map(|(item, b)| top_y.and_then(|ty| ((b.y - ty).abs() < 1.0).then_some(item)))
            .collect();
        if topmost.len() != 1 {
            return Err(drift_error(
                "The reply expansion trigger is not geometrically unique.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "video.comments.reply-template",
                    "comment panel -> local reply buttons -> unique topmost",
                ),
                json!({
                    "candidate_count": visible.len(),
                    "visible_count": visible.len(),
                    "enabled_count": 0,
                    "geometry_reason": "topmost_not_unique",
                }),
            ));
        }

        page.scroll_into_view(&topmost[0], 10_000).await?;
        let trigger = require_target(
            page,
            &topmost,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comments.reply-template",
                "comment panel -> local reply buttons -> unique topmost",
            ),
            &TargetOptions::default()
                .require_enabled()
                .scope(&panel)
                .scope_tolerance(4.0),
        )
        .await?;

        let label = page.inner_text(&trigger, 5_000).await?;
        if !REPLY_EXPAND_LABEL_RE.is_match(label.trim()) {
            return Err(drift_error(
                "The reply expansion trigger lacks local semantic evidence.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "video.comments.reply-template",
                    "reply button -> local expand/view reply label",
                ),
                json!({ "geometry_reason": "state_mismatch" }),
            ));
        }

        let attempts = max_attempts.clamp(1, 3);
        let mut last_error: Option<DoudouError> = None;
        for attempt in 1..=attempts {
            match page
                .click_element(&trigger, MouseButton::Left, 10_000)
                .await
            {
                Ok(()) => return Ok(1),
                Err(error) => {
                    last_error = Some(error);
                    if attempt < attempts {
                        page.sleep_ms(150 * (attempt as u64)).await?;
                    }
                }
            }
        }

        Err(drift_error(
            "The reply expansion trigger did not complete after bounded attempts.",
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comments.reply-template",
                "comment panel -> unique topmost reply trigger -> bounded retry",
            ),
            json!({
                "geometry_reason": "reply_trigger_action_failed",
                "attempts": attempts,
                "cause_type": last_error.map(|e| e.code),
            }),
        ))
    }
}

/// Video comment / reply composer component (`video-comment-composer`, version `2026-08-03.2`).
pub struct VideoCommentComposerComponent<'a> {
    page: &'a dyn UiPage,
    scope: Option<ElementId>,
}

impl<'a> VideoCommentComposerComponent<'a> {
    pub const NAME: &'static str = "video-comment-composer";
    pub const VERSION: &'static str = "2026-08-03.2";
    pub const CONTAINER_SELECTOR: &'static str = ".comment-input-inner-container";
    pub const EDITOR_SELECTOR: &'static str =
        r#"[contenteditable="true"][role="combobox"], textarea, input"#;
    pub const CONTROLS_SELECTOR: &'static str = ".commentInput-right-ct span";

    /// Construct a top-level comment composer bound to `page`.
    pub const fn new(page: &'a dyn UiPage) -> Self {
        Self { page, scope: None }
    }

    /// Construct a reply composer scoped to a confirmed comment item.
    pub fn scoped(page: &'a dyn UiPage, scope: ElementId) -> Self {
        Self {
            page,
            scope: Some(scope),
        }
    }

    /// Capability name (`video.comment.reply` when scoped, else `video.comment.compose`).
    pub const fn capability(&self) -> &'static str {
        if self.scope.is_some() {
            "video.comment.reply"
        } else {
            "video.comment.compose"
        }
    }

    /// Query candidate composer container elements within the configured scope.
    pub async fn candidates(&self) -> Result<Vec<ElementId>> {
        self.page
            .query_all(
                self.scope.as_ref(),
                &SelectorQuery::css(Self::CONTAINER_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await
    }

    async fn resolved_scope(&self) -> Result<Option<ElementId>> {
        if let Some(ref s) = self.scope {
            return Ok(Some(s.clone()));
        }
        page_body(self.page).await
    }

    /// Resolve the unique visible comment input container.
    pub async fn container(&self) -> Result<ElementId> {
        let candidates = self.candidates().await?;
        let scope = self.resolved_scope().await?;
        let mut opts = TargetOptions::default();
        if let Some(ref s) = scope {
            opts = opts.scope(s);
        }
        let strategy = if self.scope.is_some() {
            "confirmed comment item -> local reply input container"
        } else {
            "visible comment input container"
        };
        require_target(
            self.page,
            &candidates,
            &ComponentContext::new(Self::NAME, Self::VERSION, self.capability(), strategy),
            &opts,
        )
        .await
    }

    /// Resolve the unique Draft combobox / textarea / input editor inside the container.
    pub async fn editor(&self) -> Result<ElementId> {
        let container = self.container().await?;
        let editors = self
            .page
            .query_all(
                Some(&container),
                &SelectorQuery::css(Self::EDITOR_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let strategy = if self.scope.is_some() {
            "confirmed comment item -> local reply container -> Draft combobox"
        } else {
            "composer-local Draft combobox"
        };
        require_target(
            self.page,
            &editors,
            &ComponentContext::new(Self::NAME, Self::VERSION, self.capability(), strategy),
            &TargetOptions::default().scope(&container),
        )
        .await
    }

    async fn focus_editor(&self, editor: &ElementId) -> Result<()> {
        let mut focused = self
            .page
            .eval_on_element(editor, ACTIVE_ELEMENT_JS, None)
            .await?
            .as_bool()
            .unwrap_or(false);
        if !focused {
            let _ = click_verified_target(
                self.page,
                editor,
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "video.comment.compose",
                    "local comment editor -> stable hit target -> dynamic mouse focus",
                ),
                ClickOptions::default().without_scroll(),
            )
            .await?;
            focused = self
                .page
                .eval_on_element(editor, ACTIVE_ELEMENT_JS, None)
                .await?
                .as_bool()
                .unwrap_or(false);
        }
        if !focused {
            return Err(drift_error(
                "The verified comment editor focus action did not focus the editor.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "video.comment.compose",
                    "local comment editor -> dynamic mouse focus -> activeElement confirmation",
                ),
                json!({ "geometry_reason": "comment_editor_not_focused_after_action" }),
            ));
        }
        Ok(())
    }

    /// Activate the top-level comment composer container if its editor is not yet visible, then focus it.
    pub async fn activate(&self) -> Result<usize> {
        let container = self.container().await?;
        let raw_editors = self
            .page
            .query_all(
                Some(&container),
                &SelectorQuery::css(Self::EDITOR_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let visible = visible_candidates(self.page, &raw_editors, DEFAULT_CANDIDATE_LIMIT).await?;
        if visible.is_empty() {
            let _ = click_verified_target(
                self.page,
                &container,
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "video.comment.compose",
                    "visible comment container -> stable hit target -> local editor render",
                ),
                ClickOptions::default().without_scroll(),
            )
            .await?;
            for _ in 0..40 {
                let re_raw = self
                    .page
                    .query_all(
                        Some(&container),
                        &SelectorQuery::css(Self::EDITOR_SELECTOR),
                        DEFAULT_CANDIDATE_LIMIT,
                    )
                    .await?;
                if !visible_candidates(self.page, &re_raw, DEFAULT_CANDIDATE_LIMIT)
                    .await?
                    .is_empty()
                {
                    break;
                }
                self.page.sleep_ms(50).await?;
            }
        }
        let final_editors = self
            .page
            .query_all(
                Some(&container),
                &SelectorQuery::css(Self::EDITOR_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let editor = require_target(
            self.page,
            &final_editors,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comment.compose",
                "activated comment container -> local Draft combobox",
            ),
            &TargetOptions::default().scope(&container),
        )
        .await?;
        self.focus_editor(&editor).await?;
        Ok(1)
    }

    /// Verify the editor is empty, focus it, and insert `text`.
    pub async fn insert_text(&self, text: &str) -> Result<ElementId> {
        let editor = self.editor().await?;
        let current = self.page.inner_text(&editor, 5_000).await?;
        let stripped = current.trim_matches(|ch: char| {
            matches!(
                ch,
                '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}' | '\r' | '\n' | ' '
            )
        });
        if !stripped.is_empty() {
            return Err(DoudouError::new(
                "COMMENT_EDITOR_NOT_EMPTY",
                "The comment editor contains unplanned content.",
            ));
        }
        self.focus_editor(&editor).await?;
        self.page.insert_text(text).await?;
        self.page.sleep_ms(250).await?;
        Ok(editor)
    }

    /// Read the current text inside `editor` via DOM value/innerText evaluation.
    pub async fn composed_text(&self, editor: &ElementId) -> Result<String> {
        let value = self
            .page
            .eval_on_element(editor, COMPOSED_TEXT_JS, None)
            .await?;
        Ok(value.as_str().unwrap_or_default().to_owned())
    }

    /// Validate that the composer's rightmost local send control is unique and enabled.
    pub async fn send_control(&self) -> Result<ElementId> {
        let container = self.container().await?;
        let controls = self
            .page
            .query_all(
                Some(&container),
                &SelectorQuery::css(Self::CONTROLS_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let visible = visible_candidates(self.page, &controls, DEFAULT_CANDIDATE_LIMIT).await?;
        let mut boxes = Vec::new();
        for item in &visible {
            if let Some(b) = self.page.bounding_box(item).await? {
                boxes.push((item.clone(), b));
            }
        }
        let rightmost_x = boxes
            .iter()
            .map(|(_, b)| b.x)
            .fold(None, |acc: Option<f64>, x| {
                Some(acc.map_or(x, |curr| curr.max(x)))
            });
        let rightmost: Vec<ElementId> = boxes
            .into_iter()
            .filter_map(|(item, b)| {
                rightmost_x.and_then(|rx| ((b.x - rx).abs() < f64::EPSILON).then_some(item))
            })
            .collect();
        if rightmost.len() != 1 {
            return Err(drift_error(
                "The video comment send action is not unique.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "video.comment.send",
                    "rightmost composer-local action",
                ),
                json!({
                    "candidate_count": rightmost.len(),
                    "visible_count": visible.len(),
                    "enabled_count": 0,
                    "geometry_reason": "rightmost_not_unique",
                }),
            ));
        }
        require_target(
            self.page,
            &rightmost,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comment.send",
                "rightmost composer-local action",
            ),
            &TargetOptions::default()
                .require_enabled()
                .scope(&container)
                .peers(&controls, PeerPosition::Rightmost, 2),
        )
        .await
    }

    /// Validate the send control and publish the comment by pressing `Enter` on the editor.
    pub async fn publish(&self) -> Result<usize> {
        let _ = self.send_control().await?;
        let editor = self.editor().await?;
        self.page
            .press_element_key(&editor, "Enter", 5_000)
            .await
            .map_err(DoudouError::mark_write_started)?;
        Ok(1)
    }

    /// Run a diagnostic probe for the video comment composer container.
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
                "video.comment.compose",
                "visible local comment input container",
            ),
            exercised,
        )
        .await
    }
}

/// Comment list item resolver, reply trigger, and like control (`video-comment-item`).
pub struct CommentItemComponent<'a> {
    page: &'a dyn UiPage,
}

impl<'a> CommentItemComponent<'a> {
    pub const NAME: &'static str = "video-comment-item";
    pub const VERSION: &'static str = "2026-08-03.2";
    pub const LIST_SELECTOR: &'static str = r#"[data-e2e="comment-list"]"#;
    pub const ITEM_XPATH: &'static str = "ancestor::div[.//div[contains(@class,'comment-item-stats-container')] and .//span[normalize-space()='回复']][1]";
    pub const STATS_SELECTOR: &'static str = ".comment-item-stats-container";
    pub const LIKE_CONTROL_XPATH: &'static str =
        "ancestor-or-self::*[self::button or @role='button' or .//*[name()='svg']][1]";
    pub const LIKE_EVIDENCE_SELECTOR: &'static str =
        "svg, [aria-label*='赞'], [data-e2e*='digg' i]";

    /// Construct a comment item component bound to `page`.
    pub const fn new(page: &'a dyn UiPage) -> Self {
        Self { page }
    }

    /// Query candidate comment list containers.
    pub async fn candidates(&self) -> Result<Vec<ElementId>> {
        self.page
            .query_all(
                None,
                &SelectorQuery::css(Self::LIST_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await
    }

    /// Run a diagnostic probe for the comment list.
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
                "video.comment.target",
                "data-e2e comment-list plus local item structure",
            ),
            exercised,
        )
        .await
    }

    async fn list(&self) -> Result<ElementId> {
        let raw = self.candidates().await?;
        let lists = visible_candidates(self.page, &raw, DEFAULT_CANDIDATE_LIMIT).await?;
        if lists.len() != 1 {
            return Err(drift_error(
                "The video comment list is not uniquely visible.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "video.comment.target",
                    "data-e2e comment-list",
                ),
                json!({ "candidate_count": lists.len() }),
            ));
        }
        Ok(lists.into_iter().next().expect("checked length 1"))
    }

    /// Scroll the comment list container downward, preferring unique scrollable `.comment-mainContent`.
    pub async fn scroll_toward_end(&self, root: &ElementId) -> Result<Value> {
        self.page
            .eval_on_element(root, COMMENT_SCROLL_TOWARD_END_JS, None)
            .await
    }

    /// Locate a unique comment item matching protocol `text` (and optional `author_name`) via bounded scroll.
    pub async fn target(
        &self,
        text: &str,
        author_name: Option<&str>,
        attempts: usize,
    ) -> Result<ElementId> {
        let root = self.list().await?;
        let visible_text = visible_comment_text(text);
        for _ in 0..attempts {
            let matches = self
                .page
                .query_all(
                    Some(&root),
                    &SelectorQuery::text(&visible_text, TextMatchMode::Exact),
                    DEFAULT_CANDIDATE_LIMIT,
                )
                .await?;
            let mut items = Vec::new();
            for candidate in
                visible_candidates(self.page, &matches, DEFAULT_CANDIDATE_LIMIT).await?
            {
                let item_matches = self
                    .page
                    .query_all(Some(&candidate), &SelectorQuery::xpath(Self::ITEM_XPATH), 5)
                    .await?;
                if item_matches.len() == 1 && self.page.is_visible(&item_matches[0]).await? {
                    let item = &item_matches[0];
                    if let Some(author) = author_name {
                        let author_hits = self
                            .page
                            .query_all(
                                Some(item),
                                &SelectorQuery::text(author, TextMatchMode::Exact),
                                5,
                            )
                            .await?;
                        if author_hits.is_empty() {
                            continue;
                        }
                    }
                    items.push(item.clone());
                }
            }

            if items.len() == 1 {
                return require_target(
                    self.page,
                    &items,
                    &ComponentContext::new(
                        Self::NAME,
                        Self::VERSION,
                        "video.comment.target",
                        "comment-list exact text plus author -> local item",
                    ),
                    &TargetOptions::default().scope(&root),
                )
                .await;
            }
            if items.len() > 1 {
                return Err(drift_error(
                    "The protocol-selected comment is ambiguous.",
                    &ComponentContext::new(
                        Self::NAME,
                        Self::VERSION,
                        "video.comment.target",
                        "comment-list text plus author",
                    ),
                    json!({ "candidate_count": items.len() }),
                ));
            }

            let scroll = self.scroll_toward_end(&root).await?;
            let resolved = scroll
                .get("resolved")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !resolved {
                return Err(drift_error(
                    "The video comment scroll root is not uniquely resolved.",
                    &ComponentContext::new(
                        Self::NAME,
                        Self::VERSION,
                        "video.comment.target",
                        "comment-list -> unique local or containing scroll root",
                    ),
                    json!({
                        "geometry_reason": scroll.get("reason").cloned().unwrap_or(Value::Null),
                        "local_count": scroll.get("local_count").cloned().unwrap_or(Value::Null),
                        "ancestor_count": scroll.get("ancestor_count").cloned().unwrap_or(Value::Null),
                        "preferred_count": scroll.get("preferred_count").cloned().unwrap_or(Value::Null),
                        "preferred_scrollable_count": scroll.get("preferred_scrollable_count").cloned().unwrap_or(Value::Null),
                    }),
                ));
            }
            let moved = scroll
                .get("moved")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !moved {
                break;
            }
            self.page.sleep_ms(350).await?;
        }

        Err(DoudouError::new(
            "COMMENT_TARGET_NOT_VISIBLE",
            "The protocol-selected comment was not found by bounded scrolling.",
        )
        .with_details(json!({
            "component": Self::NAME,
            "component_version": Self::VERSION,
            "capability": "video.comment.target",
            "strategy": "comment-list bounded scroll plus exact normalized text",
            "attempts": attempts,
        })))
    }

    /// Click the local `"回复"` action on `item` and wait for its local reply composer to mount.
    pub async fn reply(&self, item: &ElementId) -> Result<usize> {
        let reply_label = VIDEO_ADAPTER.label("reply").unwrap_or("回复");
        let actions = self
            .page
            .query_all(
                Some(item),
                &SelectorQuery::text(reply_label, TextMatchMode::Exact),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let action = require_target(
            self.page,
            &actions,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comment.reply",
                "comment-item-local semantic reply",
            ),
            &TargetOptions::default().require_enabled().scope(item),
        )
        .await?;
        self.page
            .click_element(&action, MouseButton::Left, 10_000)
            .await?;

        for _ in 0..50 {
            let containers = self
                .page
                .query_all(
                    Some(item),
                    &SelectorQuery::css(VideoCommentComposerComponent::CONTAINER_SELECTOR),
                    DEFAULT_CANDIDATE_LIMIT,
                )
                .await?;
            let visible =
                visible_candidates(self.page, &containers, DEFAULT_CANDIDATE_LIMIT).await?;
            if visible.len() == 1 {
                let _ = require_target(
                    self.page,
                    &containers,
                    &ComponentContext::new(
                        Self::NAME,
                        Self::VERSION,
                        "video.comment.reply",
                        "confirmed comment item -> local reply action -> local reply composer",
                    ),
                    &TargetOptions::default().scope(item),
                )
                .await?;
                return Ok(1);
            }
            if visible.len() > 1 {
                break;
            }
            self.page.sleep_ms(100).await?;
        }

        let containers = self
            .page
            .query_all(
                Some(item),
                &SelectorQuery::css(VideoCommentComposerComponent::CONTAINER_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let visible = visible_candidates(self.page, &containers, DEFAULT_CANDIDATE_LIMIT).await?;
        Err(drift_error(
            "The confirmed comment did not expose one local reply composer.",
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comment.reply",
                "confirmed comment item -> local reply action -> local reply composer",
            ),
            json!({
                "candidate_count": containers.len().min(DEFAULT_CANDIDATE_LIMIT),
                "visible_count": visible.len(),
                "geometry_reason": "reply_composer_not_unique",
            }),
        ))
    }

    /// Resolve and verify the like button for `item` matching protocol `count` and unliked state.
    pub async fn like_control(&self, item: &ElementId, count: u64) -> Result<ElementId> {
        let stats_candidates = self
            .page
            .query_all(
                Some(item),
                &SelectorQuery::css(Self::STATS_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let stats = require_target(
            self.page,
            &stats_candidates,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comment.like",
                "comment-item-local stats container",
            ),
            &TargetOptions::default().scope(item).scope_tolerance(4.0),
        )
        .await?;

        let count_text = count.to_string();
        let count_labels = self
            .page
            .query_all(
                Some(&stats),
                &SelectorQuery::text(&count_text, TextMatchMode::Exact),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let count_label = require_target(
            self.page,
            &count_labels,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comment.like",
                "stats-local exact protocol count label",
            ),
            &TargetOptions::default().scope(&stats),
        )
        .await?;

        let controls = self
            .page
            .query_all(
                Some(&count_label),
                &SelectorQuery::xpath(Self::LIKE_CONTROL_XPATH),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let control = require_target(
            self.page,
            &controls,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comment.like",
                "protocol count label -> local icon/button action",
            ),
            &TargetOptions::default().require_enabled().scope(&stats),
        )
        .await?;

        let evidence = self
            .page
            .query_all(
                Some(&control),
                &SelectorQuery::css(Self::LIKE_EVIDENCE_SELECTOR),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let _ = require_target(
            self.page,
            &evidence,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "video.comment.like",
                "local like action -> semantic/icon evidence",
            ),
            &TargetOptions::default().scope(&control),
        )
        .await?;

        let pressed = self.page.get_attribute(&control, "aria-pressed").await?;
        if pressed.as_deref() == Some("true") {
            return Err(drift_error(
                "The page like state contradicts the unliked protocol plan.",
                &ComponentContext::new(
                    Self::NAME,
                    Self::VERSION,
                    "video.comment.like",
                    "local like action -> unliked state verification",
                ),
                json!({ "geometry_reason": "state_mismatch" }),
            ));
        }

        Ok(control)
    }

    /// Click the verified like button on `item`.
    pub async fn like(&self, item: &ElementId, count: u64) -> Result<usize> {
        let control = self.like_control(item, count).await?;
        self.page
            .click_element(&control, MouseButton::Left, 10_000)
            .await
            .map_err(DoudouError::mark_write_started)?;
        Ok(1)
    }
}
