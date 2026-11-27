//! Versioned Douyin page adapters (routes, CSS selectors, and semantic labels).

use std::collections::BTreeMap;

use doudou_core::{DoudouError, Result};
use serde_json::json;

use crate::page::{ElementId, SelectorQuery, TextMatchMode, UiPage};

/// Versioned contract binding a Douyin web route to its CSS selectors and UI labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageAdapter {
    pub name: &'static str,
    pub version: &'static str,
    pub route_prefix: &'static str,
    pub route_aliases: &'static [&'static str],
    pub selectors: &'static [(&'static str, &'static str)],
    pub labels: &'static [(&'static str, &'static str)],
}

impl PageAdapter {
    /// Return `true` when `path` begins with `route_prefix` or any registered alias.
    pub fn matches_route(&self, path: &str) -> bool {
        path.starts_with(self.route_prefix)
            || self
                .route_aliases
                .iter()
                .any(|alias| path.starts_with(alias))
    }

    /// Look up a named CSS selector in this adapter.
    pub fn selector(&self, key: &str) -> Option<&'static str> {
        self.selectors
            .iter()
            .find_map(|(k, v)| (*k == key).then_some(*v))
    }

    /// Look up a named UI text label in this adapter.
    pub fn label(&self, key: &str) -> Option<&'static str> {
        self.labels
            .iter()
            .find_map(|(k, v)| (*k == key).then_some(*v))
    }

    /// Query elements matching the named selector key in `page`.
    pub async fn query_selector(
        &self,
        page: &dyn UiPage,
        key: &str,
        limit: usize,
    ) -> Result<Vec<ElementId>> {
        let Some(selector) = self.selector(key) else {
            return Err(DoudouError::new(
                "PAGE_ADAPTER_DRIFTED",
                format!("Adapter '{}' has no selector key '{key}'.", self.name),
            )
            .with_details(json!({
                "adapter": self.name,
                "adapter_version": self.version,
                "missing_selector_key": key,
            })));
        };
        page.query_all(None, &SelectorQuery::css(selector), limit)
            .await
    }

    /// Query elements matching the named label key in `page`.
    pub async fn query_label(
        &self,
        page: &dyn UiPage,
        key: &str,
        mode: TextMatchMode,
        limit: usize,
    ) -> Result<Vec<ElementId>> {
        let Some(label) = self.label(key) else {
            return Err(DoudouError::new(
                "PAGE_ADAPTER_DRIFTED",
                format!("Adapter '{}' has no label key '{key}'.", self.name),
            )
            .with_details(json!({
                "adapter": self.name,
                "adapter_version": self.version,
                "missing_label_key": key,
            })));
        };
        page.query_all(None, &SelectorQuery::text(label, mode), limit)
            .await
    }

    /// Require that every selector in `keys` resolves to at least one visible control.
    pub async fn require(
        &self,
        page: &dyn UiPage,
        capability: &str,
        keys: &[&str],
    ) -> Result<BTreeMap<String, usize>> {
        let mut missing = Vec::new();
        let mut counts = BTreeMap::new();
        for &key in keys {
            let candidates = self.query_selector(page, key, 20).await?;
            let mut visible = 0usize;
            for element in candidates.iter().take(20) {
                if page.is_visible(element).await? {
                    visible += 1;
                }
            }
            counts.insert(key.to_owned(), visible);
            if visible == 0 {
                missing.push(key.to_owned());
            }
        }
        if !missing.is_empty() {
            let current_url = page.url().await?;
            let page_path = current_url
                .split('?')
                .next()
                .unwrap_or(&current_url)
                .to_owned();
            return Err(DoudouError::new(
                "PAGE_ADAPTER_DRIFTED",
                "The versioned page adapter no longer matches the controls required for this write.",
            )
            .with_details(json!({
                "adapter": self.name,
                "adapter_version": self.version,
                "capability": capability,
                "page_path": page_path,
                "missing_controls": missing,
                "visible_counts": counts,
            })));
        }
        Ok(counts)
    }
}

/// Versioned adapter for `https://www.douyin.com/chat`.
pub const CHAT_ADAPTER: PageAdapter = PageAdapter {
    name: "douyin-chat",
    version: "2026-08-03.1",
    route_prefix: "/chat",
    route_aliases: &[],
    selectors: &[
        (
            "composer",
            r#"[data-e2e="msg-input"] [contenteditable="true"]"#,
        ),
        ("message_item", r#"[data-e2e="msg-item-content"]"#),
        ("image_upload", r#"[role="button"].semi-upload-add"#),
        ("send_button", "svg.e2e-send-msg-btn"),
        (
            "conversation_list",
            r#"div.conversationConversationListwrapper:has([data-e2e="conversation-item"])"#,
        ),
        ("session_candidates", r#"[data-e2e="conversation-item"]"#),
        ("session_avatars", "img"),
        ("message_context_menu", ".MessageOperatePopWindowwrapper"),
        ("message_context_action", ".MessageOperatePopBodybuttonItem"),
        (
            "message_emoji_toggle",
            ".PopBodyEmojiQuickReplytoggleButton",
        ),
        ("message_emoji_like", r#"img[title="[爱心]"]"#),
        (
            "image_preview_dialog",
            r#"[role="dialog"].semi-modal-content:has(.MsgInputSendFileModalbox img)"#,
        ),
        ("image_preview_send", "button.MsgInputSendFileModalbtnSure"),
    ],
    labels: &[("reply", "回复"), ("send", "发送"), ("like", "点赞")],
};

/// Versioned adapter for `https://www.douyin.com/video/*` and `/note/*`.
pub const VIDEO_ADAPTER: PageAdapter = PageAdapter {
    name: "douyin-video",
    version: "2026-08-03.1",
    route_prefix: "/video/",
    route_aliases: &["/note/"],
    selectors: &[
        ("render_data", "#RENDER_DATA"),
        ("inventory_controls", "button, [role=button], [data-e2e]"),
        (
            "comment_button",
            r#"[data-e2e="feed-comment-icon"], [data-e2e*="comment" i], button[aria-label*="评论"], [title*="评论"]"#,
        ),
        ("comment_item", r#"[data-e2e="comment-item"]"#),
        ("comment_container", ".comment-input-inner-container"),
        (
            "comment_placeholder",
            ".public-DraftEditorPlaceholder-root:visible",
        ),
        ("comment_send_controls", ".commentInput-right-ct span"),
        (
            "comment_editor",
            r#"textarea, input, [contenteditable="true"]"#,
        ),
        (
            "fallback_editor",
            r#"[contenteditable="true"][role="combobox"]:visible"#,
        ),
        ("verification_mask", ".second-verify-mask"),
        ("like_count", "p"),
    ],
    labels: &[("tutorial", "我知道了"), ("reply", "回复")],
};

/// Versioned adapter for `https://www.douyin.com/user/*`.
pub const PROFILE_ADAPTER: PageAdapter = PageAdapter {
    name: "douyin-profile",
    version: "2026-08-02.1",
    route_prefix: "/user/",
    route_aliases: &[],
    selectors: &[("scroll_container", ".route-scroll-container")],
    labels: &[],
};

/// Versioned adapter for authentication and verification challenge detection.
pub const AUTH_ADAPTER: PageAdapter = PageAdapter {
    name: "douyin-auth",
    version: "2026-08-03.1",
    route_prefix: "/",
    route_aliases: &[],
    selectors: &[
        (
            "slider",
            r#"iframe[src*="captcha" i], [class*="captcha" i] [class*="slide" i], [class*="captcha" i] [class*="drag" i]"#,
        ),
        (
            "sms_code",
            r#".second-verify-mask input, input[placeholder*="验证码"], input[maxlength="6"]"#,
        ),
        (
            "qr_login",
            r#"#douyin_login_comp_scan_code_qrcode img, [id*="qrcode" i] img, [class*="login" i] img[src*="qr" i], [class*="login" i] canvas[class*="qr" i], [class*="qrcode" i] img, [class*="qr-code" i] img"#,
        ),
        (
            "login_panel",
            "#douyin_login_comp_flat_panel, #login-panel-new, #douyin-login-new-id",
        ),
        (
            "phone_input",
            r#"input[name="normal-input"][placeholder*="手机号"], input[placeholder*="手机号"]"#,
        ),
        ("sms_tab", "#douyin_login_comp_mobile_title"),
        ("sms_submit", "#douyin_login_comp_btn_id"),
        (
            "passkey",
            r#"[role="dialog"] button:has-text("Passkey"), [role="dialog"] button:has-text("通行密钥"), [class*="login" i] button:has-text("Passkey"), [class*="login" i] button:has-text("通行密钥")"#,
        ),
        (
            "generic_verification",
            r#"iframe[src*="verify" i], .second-verify-mask:has(input), [role="dialog"][class*="verify" i]:has(button)"#,
        ),
    ],
    labels: &[
        ("quick_login", "快捷登录"),
        ("get_code", "获取验证码"),
        ("send_code", "发送验证码"),
        ("login", "登录"),
        ("confirm", "确定"),
        ("verify", "验证"),
        ("refresh_qr", "刷新"),
    ],
};

/// All registered versioned Douyin page adapters.
pub const ADAPTERS: [&PageAdapter; 4] = [
    &AUTH_ADAPTER,
    &CHAT_ADAPTER,
    &PROFILE_ADAPTER,
    &VIDEO_ADAPTER,
];
