//! Login, captcha, SMS, passkey, and generic verification challenge detection.

use std::collections::BTreeSet;

use doudou_core::Result;
use serde_json::{Value, json};

use crate::adapters::{AUTH_ADAPTER, PageAdapter};
use crate::geometry::{
    ComponentContext, DEFAULT_CANDIDATE_LIMIT, TargetOptions, drift_error, require_target,
    resolution_probe, visible_candidates,
};
use crate::page::{ElementId, MouseButton, SelectorQuery, TextMatchMode, UiPage};

const GENERIC_VERIFICATION_MARKERS: [&str; 5] = ["验证", "验证码", "安全", "滑块", "拖动"];

/// Detects and interacts with Douyin authentication and security verification overlays.
pub struct AuthChallengeComponent<'a> {
    page: &'a dyn UiPage,
    adapter: &'static PageAdapter,
}

impl<'a> AuthChallengeComponent<'a> {
    pub const NAME: &'static str = "auth-challenge";
    pub const VERSION: &'static str = "2026-08-03.1";
    pub const CONTAINER_XPATH: &'static str = "ancestor-or-self::*[@role='dialog' or contains(@class,'verify') or contains(@class,'captcha') or contains(@class,'login') or contains(@class,'second-verify-mask')][1]";
    pub const KINDS: [&'static str; 5] = [
        "sms_code",
        "slider",
        "qr_login",
        "passkey",
        "generic_verification",
    ];

    /// Construct an auth challenge detector using the default [`AUTH_ADAPTER`].
    pub const fn new(page: &'a dyn UiPage) -> Self {
        Self {
            page,
            adapter: &AUTH_ADAPTER,
        }
    }

    /// Construct an auth challenge detector with an explicit [`PageAdapter`].
    pub const fn with_adapter(page: &'a dyn UiPage, adapter: &'static PageAdapter) -> Self {
        Self { page, adapter }
    }

    async fn body(&self) -> Result<Option<ElementId>> {
        let nodes = self
            .page
            .query_all(None, &SelectorQuery::css("body"), 1)
            .await?;
        Ok(nodes.into_iter().next())
    }

    async fn container(&self, challenge: &ElementId, capability: &str) -> Result<ElementId> {
        let candidates = self
            .page
            .query_all(
                Some(challenge),
                &SelectorQuery::xpath(Self::CONTAINER_XPATH),
                DEFAULT_CANDIDATE_LIMIT,
            )
            .await?;
        let body = self.body().await?;
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
                capability,
                "semantic auth challenge -> local dialog/mask container",
            ),
            &opts,
        )
        .await
    }

    async fn challenge(&self, kind: &str) -> Result<(ElementId, ElementId)> {
        let capability = format!("auth.{kind}");
        let strategy = format!("versioned semantic {kind} candidate");
        let candidates = self
            .adapter
            .query_selector(self.page, kind, DEFAULT_CANDIDATE_LIMIT)
            .await?;
        let challenge = require_target(
            self.page,
            &candidates,
            &ComponentContext::new(Self::NAME, Self::VERSION, &capability, &strategy),
            &TargetOptions::default(),
        )
        .await?;
        let container = self.container(&challenge, &capability).await?;
        if kind == "generic_verification" {
            let text = self
                .page
                .inner_text(&container, 1_000)
                .await?
                .to_lowercase();
            let tag_name = self
                .page
                .eval_on_element(&challenge, "element => element.tagName", None)
                .await?
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let has_marker = GENERIC_VERIFICATION_MARKERS
                .iter()
                .any(|marker| text.contains(marker));
            if tag_name != "IFRAME" && !has_marker {
                return Err(drift_error(
                    "The generic auth candidate lacks local verification evidence.",
                    &ComponentContext::new(
                        Self::NAME,
                        Self::VERSION,
                        "auth.generic_verification",
                        "generic auth candidate -> local verification evidence",
                    ),
                    json!({ "geometry_reason": "state_mismatch" }),
                ));
            }
        }
        Ok((challenge, container))
    }

    /// Detect the highest-priority visible authentication or verification challenge.
    pub async fn detect(&self) -> Result<Option<String>> {
        for kind in Self::KINDS {
            let raw = self
                .adapter
                .query_selector(self.page, kind, DEFAULT_CANDIDATE_LIMIT)
                .await?;
            let visible = visible_candidates(self.page, &raw, DEFAULT_CANDIDATE_LIMIT).await?;
            if visible.is_empty() {
                continue;
            }
            let container = match self.challenge(kind).await {
                Ok((_challenge, container)) => container,
                Err(error) => {
                    let is_empty_drift = error.code == "PAGE_ADAPTER_DRIFTED"
                        && error
                            .details
                            .as_ref()
                            .and_then(|d| d.get("candidate_count"))
                            .and_then(Value::as_u64)
                            == Some(0)
                        && error
                            .details
                            .as_ref()
                            .and_then(|d| d.get("visible_count"))
                            .and_then(Value::as_u64)
                            == Some(0);
                    if is_empty_drift {
                        continue;
                    }
                    return Err(error);
                }
            };

            if kind == "sms_code"
                && let Some(quick_label) = self.adapter.label("quick_login")
            {
                let quick_raw = self
                    .page
                    .query_all(
                        Some(&container),
                        &SelectorQuery::text(quick_label, TextMatchMode::Substring),
                        5,
                    )
                    .await?;
                let quick = visible_candidates(self.page, &quick_raw, 5).await?;
                if quick.len() > 1 {
                    let _ = require_target(
                        self.page,
                        &quick_raw,
                        &ComponentContext::new(
                            Self::NAME,
                            Self::VERSION,
                            "auth.quick_login_sms",
                            "SMS dialog -> quick-login semantic label",
                        ),
                        &TargetOptions::default().scope(&container),
                    )
                    .await?;
                }
                if quick.len() == 1 {
                    return Ok(Some("quick_login_sms".to_owned()));
                }
            }

            return Ok(Some(kind.to_owned()));
        }
        Ok(None)
    }

    /// Click the get/send SMS code action when an SMS challenge is active.
    pub async fn request_sms_code(&self) -> Result<bool> {
        let Some(kind) = self.detect().await? else {
            return Ok(false);
        };
        if kind != "sms_code" && kind != "quick_login_sms" {
            return Ok(false);
        }
        let sms_raw = self
            .adapter
            .query_selector(self.page, "sms_code", DEFAULT_CANDIDATE_LIMIT)
            .await?;
        let challenge = require_target(
            self.page,
            &sms_raw,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "auth.sms_code",
                "versioned SMS input candidate",
            ),
            &TargetOptions::default(),
        )
        .await?;
        let container = self.container(&challenge, "auth.sms_code.request").await?;

        let mut controls = Vec::new();
        let mut seen = BTreeSet::new();
        for key in ["get_code", "send_code"] {
            if let Some(label) = self.adapter.label(key) {
                let found = self
                    .page
                    .query_all(
                        Some(&container),
                        &SelectorQuery::text(label, TextMatchMode::Substring),
                        DEFAULT_CANDIDATE_LIMIT,
                    )
                    .await?;
                for id in found {
                    if seen.insert(id.clone()) {
                        controls.push(id);
                    }
                }
            }
        }

        let control = require_target(
            self.page,
            &controls,
            &ComponentContext::new(
                Self::NAME,
                Self::VERSION,
                "auth.sms_code.request",
                "SMS dialog -> local get/send-code action",
            ),
            &TargetOptions::default().require_enabled().scope(&container),
        )
        .await?;
        self.page
            .click_element(&control, MouseButton::Left, 10_000)
            .await?;
        Ok(true)
    }

    /// Run diagnostic probes across all registered auth challenge kinds.
    pub async fn probe(&self, exercised: bool) -> Result<Vec<Value>> {
        let mut reports = Vec::with_capacity(Self::KINDS.len());
        for kind in Self::KINDS {
            let visible = if exercised {
                let raw = self
                    .adapter
                    .query_selector(self.page, kind, DEFAULT_CANDIDATE_LIMIT)
                    .await?;
                !visible_candidates(self.page, &raw, DEFAULT_CANDIDATE_LIMIT)
                    .await?
                    .is_empty()
            } else {
                false
            };
            let capability = format!("auth.{kind}");
            let strategy = format!("versioned semantic {kind} -> local auth container");
            let report = resolution_probe(
                self.page,
                || async {
                    let (_challenge, container) = self.challenge(kind).await?;
                    Ok(container)
                },
                &ComponentContext::new(Self::NAME, Self::VERSION, &capability, &strategy),
                visible,
                "challenge_not_present",
            )
            .await?;
            reports.push(report);
        }
        Ok(reports)
    }
}
