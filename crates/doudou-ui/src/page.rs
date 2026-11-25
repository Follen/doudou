//! Object-safe page, session, and network-capture traits for UI components.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use doudou_core::{Result, sha256_bytes};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

/// Opaque DOM element handle returned by a [`UiPage`] implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ElementId(pub String);

impl ElementId {
    /// Create a new opaque element identifier.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the underlying identifier string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ElementId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for ElementId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Axis-aligned bounding rectangle in viewport CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl BoundingBox {
    /// Construct a bounding box from top-left coordinates and dimensions.
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Return `true` when coordinates are finite and both width and height are positive.
    pub fn is_valid(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }

    /// Return the geometric center `(x, y)` of this rectangle.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Return `true` when `target` fits within `self` expanded by `tolerance` pixels on every edge.
    pub fn contains(&self, target: &Self, tolerance: f64) -> bool {
        target.x >= self.x - tolerance
            && target.y >= self.y - tolerance
            && target.x + target.width <= self.x + self.width + tolerance
            && target.y + target.height <= self.y + self.height + tolerance
    }

    /// Return `true` when `(px, py)` lies within this rectangle (inclusive of edges).
    pub fn contains_point(&self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.x + self.width && py >= self.y && py <= self.y + self.height
    }

    /// Return the maximum absolute coordinate or size delta across all four fields.
    pub fn max_delta(&self, other: &Self) -> f64 {
        (self.x - other.x)
            .abs()
            .max((self.y - other.y).abs())
            .max((self.width - other.width).abs())
            .max((self.height - other.height).abs())
    }
}

/// Blocker metadata returned when an element is occluded at its click point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockerInfo {
    pub tag: String,
    pub class_name: String,
}

/// Result of a viewport `document.elementFromPoint(x, y)` hit test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointHit {
    /// Topmost DOM element hit at `(x, y)`, if known.
    pub element_id: Option<ElementId>,
    /// Uppercase HTML/SVG tag name of the topmost element.
    pub tag: String,
    /// `className` of the topmost element (truncated to 160 chars).
    pub class_name: String,
    /// Ancestor chain from the topmost hit element walking up `parentElement` (inclusive).
    #[serde(default)]
    pub ancestor_ids: Vec<ElementId>,
    /// Whether the topmost element is marked non-interactive/decorative.
    #[serde(default)]
    pub non_interactive: bool,
}

impl PointHit {
    /// Return `true` when clicking this point reaches `target` (either directly or via
    /// ancestor-chain containment from a descendant inside `target`).
    pub fn receives_events_for(&self, target: &ElementId) -> bool {
        if self.element_id.as_ref() == Some(target) {
            return true;
        }
        self.ancestor_ids.iter().any(|ancestor| ancestor == target)
    }

    /// Convert this hit into [`BlockerInfo`] when it occludes another element.
    pub fn as_blocker(&self) -> BlockerInfo {
        let truncated: String = self.class_name.chars().take(160).collect();
        BlockerInfo {
            tag: self.tag.clone(),
            class_name: truncated,
        }
    }
}

/// Mouse button for pointer click operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
}

/// Text matching precision for DOM text queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMatchMode {
    Exact,
    Substring,
}

/// DOM query descriptor executed against either the document or a scoped ancestor element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorQuery {
    Css(String),
    XPath(String),
    Text { text: String, mode: TextMatchMode },
}

impl SelectorQuery {
    /// Create a CSS selector query.
    pub fn css(selector: impl Into<String>) -> Self {
        Self::Css(selector.into())
    }

    /// Create an `XPath` query (without a leading `xpath=` prefix).
    pub fn xpath(expr: impl Into<String>) -> Self {
        Self::XPath(expr.into())
    }

    /// Create a text match query.
    pub fn text(text: impl Into<String>, mode: TextMatchMode) -> Self {
        Self::Text {
            text: text.into(),
            mode,
        }
    }
}

/// How [`CaptureFilter`] compares request/response URLs against configured paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathMatchMode {
    Contains,
    Exact,
}

/// Filter describing which network endpoints should be recorded during a UI write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFilter {
    pub paths: Vec<String>,
    pub match_mode: PathMatchMode,
}

impl CaptureFilter {
    /// Match any URL containing at least one of `paths`.
    pub fn contains<I, S>(paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            paths: paths.into_iter().map(Into::into).collect(),
            match_mode: PathMatchMode::Contains,
        }
    }

    /// Match only URLs whose normalized path equals one of `paths`.
    pub fn exact<I, S>(paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            paths: paths.into_iter().map(Into::into).collect(),
            match_mode: PathMatchMode::Exact,
        }
    }

    /// Return `true` when `raw_url` matches this filter.
    pub fn matches(&self, raw_url: &str) -> bool {
        match self.match_mode {
            PathMatchMode::Contains => self
                .paths
                .iter()
                .any(|path| raw_url.contains(path.as_str())),
            PathMatchMode::Exact => {
                let actual_path = Url::parse(raw_url).map_or_else(
                    |_| raw_url.split('?').next().unwrap_or(raw_url).to_owned(),
                    |u| u.path().to_owned(),
                );
                let trimmed = actual_path.trim_end_matches('/');
                self.paths
                    .iter()
                    .any(|path| trimmed == path.trim_end_matches('/'))
            }
        }
    }
}

/// Sanitized endpoint metadata recorded in write receipts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSummary {
    pub origin: String,
    pub path: String,
    pub query_keys: Vec<String>,
}

/// Produce a deterministic [`EndpointSummary`] matching Python's `endpoint_summary()`.
pub fn endpoint_summary(raw_url: &str) -> EndpointSummary {
    if let Ok(parsed) = Url::parse(raw_url) {
        let mut netloc = parsed.host_str().unwrap_or_default().to_owned();
        if let Some(port) = parsed.port() {
            netloc.push(':');
            netloc.push_str(&port.to_string());
        }
        let query_keys: Vec<String> = parsed
            .query_pairs()
            .map(|(k, _)| k.into_owned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        return EndpointSummary {
            origin: format!("{}://{netloc}", parsed.scheme()),
            path: parsed.path().to_owned(),
            query_keys,
        };
    }
    let path = raw_url.split('?').next().unwrap_or(raw_url).to_owned();
    EndpointSummary {
        origin: "://".to_owned(),
        path,
        query_keys: Vec::new(),
    }
}

/// Captured outgoing HTTP request during a UI write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedRequest {
    pub method: String,
    pub url: String,
    pub path: String,
    pub endpoint: EndpointSummary,
    pub body: Vec<u8>,
    pub bytes: usize,
    pub sha256: String,
}

impl CapturedRequest {
    /// Build a captured request record with computed length, digest, and endpoint summary.
    pub fn new(method: impl Into<String>, url: impl Into<String>, body: Vec<u8>) -> Self {
        let url = url.into();
        let endpoint = endpoint_summary(&url);
        let path = endpoint.path.clone();
        let bytes = body.len();
        let sha256 = sha256_bytes(&body);
        Self {
            method: method.into(),
            url,
            path,
            endpoint,
            body,
            bytes,
            sha256,
        }
    }

    /// Serialize request metadata in the canonical write-receipt schema.
    pub fn to_metadata_json(&self) -> Value {
        json!({
            "method": self.method,
            "endpoint": self.endpoint,
            "bytes": self.bytes,
            "sha256": self.sha256,
        })
    }
}

/// Captured HTTP response during a UI write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedResponse {
    pub url: String,
    pub path: String,
    pub endpoint: EndpointSummary,
    pub http_status: u16,
    pub body: Vec<u8>,
    pub bytes: usize,
    pub sha256: String,
}

impl CapturedResponse {
    /// Build a captured response record with computed length, digest, and endpoint summary.
    pub fn new(url: impl Into<String>, http_status: u16, body: Vec<u8>) -> Self {
        let url = url.into();
        let endpoint = endpoint_summary(&url);
        let path = endpoint.path.clone();
        let bytes = body.len();
        let sha256 = sha256_bytes(&body);
        Self {
            url,
            path,
            endpoint,
            http_status,
            body,
            bytes,
            sha256,
        }
    }

    /// Serialize response metadata in the canonical write-receipt schema.
    pub fn to_metadata_json(&self) -> Value {
        json!({
            "endpoint": self.endpoint,
            "http_status": self.http_status,
            "bytes": self.bytes,
            "sha256": self.sha256,
        })
    }
}

/// Snapshot of captured network traffic collected by a [`NetworkCaptureHandle`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkCaptureSnapshot {
    pub requests: Vec<CapturedRequest>,
    pub responses: Vec<CapturedResponse>,
}

impl NetworkCaptureSnapshot {
    /// Convert captured requests and responses into the receipt `network` JSON object.
    pub fn to_network_json(&self) -> Value {
        let requests: Vec<Value> = self
            .requests
            .iter()
            .map(CapturedRequest::to_metadata_json)
            .collect();
        let responses: Vec<Value> = self
            .responses
            .iter()
            .map(CapturedResponse::to_metadata_json)
            .collect();
        json!({
            "requests": requests,
            "responses": responses,
        })
    }
}

/// Active network capture session returned by [`UiPage::start_network_capture`].
#[async_trait]
pub trait NetworkCaptureHandle: Send + Sync {
    /// Inspect captured traffic collected so far without stopping the listener.
    async fn snapshot(&self) -> Result<NetworkCaptureSnapshot>;

    /// Flush pending response bodies, detach listeners, and return all captured traffic.
    async fn stop(self: Box<Self>) -> Result<NetworkCaptureSnapshot>;
}

/// Object-safe browser page abstraction decoupleable from CDP or Playwright backends.
#[async_trait]
pub trait UiPage: Send + Sync {
    /// Current main-frame URL.
    async fn url(&self) -> Result<String>;

    /// Navigate to `url` and wait until DOM content is loaded.
    async fn goto(&self, url: &str) -> Result<()>;

    /// Pause execution for `ms` milliseconds.
    async fn sleep_ms(&self, ms: u64) -> Result<()>;

    /// Monotonic timestamp in milliseconds for bounded wait loops.
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }

    /// Query matching DOM elements within the document (`scope = None`) or a scoped element.
    async fn query_all(
        &self,
        scope: Option<&ElementId>,
        query: &SelectorQuery,
        limit: usize,
    ) -> Result<Vec<ElementId>>;

    /// Return `true` when `element` is visibly rendered.
    async fn is_visible(&self, element: &ElementId) -> Result<bool>;

    /// Return `true` when `element` is interactive / not disabled.
    async fn is_enabled(&self, element: &ElementId) -> Result<bool>;

    /// Return `true` when `element` is non-interactive decorative chrome
    /// (such as `aria-hidden="true"`, `role="presentation"`, `role="none"`, or `pointer-events: none`).
    async fn is_decorative(&self, element: &ElementId) -> Result<bool> {
        let aria_hidden = self.get_attribute(element, "aria-hidden").await?;
        if aria_hidden.as_deref() == Some("true") {
            return Ok(true);
        }
        let role = self.get_attribute(element, "role").await?;
        Ok(matches!(role.as_deref(), Some("presentation" | "none")))
    }

    /// Measure the element's current viewport bounding box.
    async fn bounding_box(&self, element: &ElementId) -> Result<Option<BoundingBox>>;

    /// Scroll `element` into view if needed within `timeout_ms`.
    async fn scroll_into_view(&self, element: &ElementId, timeout_ms: u64) -> Result<()>;

    /// Read `element.innerText` within `timeout_ms`.
    async fn inner_text(&self, element: &ElementId, timeout_ms: u64) -> Result<String>;

    /// Read `element.textContent` within `timeout_ms`.
    async fn text_content(&self, element: &ElementId, timeout_ms: u64) -> Result<Option<String>>;

    /// Read an HTML attribute value on `element`.
    async fn get_attribute(&self, element: &ElementId, name: &str) -> Result<Option<String>>;

    /// Evaluate JavaScript in the page context and deserialize the JSON result.
    async fn eval_json(&self, script: &str, arg: Option<&Value>) -> Result<Value>;

    /// Evaluate a JS function `(element, arg) => ...` against a specific element handle.
    async fn eval_on_element(
        &self,
        element: &ElementId,
        script: &str,
        arg: Option<&Value>,
    ) -> Result<Value>;

    /// Inspect the topmost hit element (and its ancestor chain) at viewport coordinates `(x, y)`.
    async fn element_from_point(&self, x: f64, y: f64) -> Result<Option<PointHit>>;

    /// Dispatch a pointer click at viewport coordinates `(x, y)`.
    ///
    /// Implementations should return a `DoudouError` with code `"CLICK_TIMEOUT"` if the click
    /// dispatch itself times out so callers can defer to state postcondition checks.
    async fn click_xy(&self, x: f64, y: f64, button: MouseButton, timeout_ms: u64) -> Result<()>;

    /// Click a resolved DOM element directly with `button`.
    async fn click_element(
        &self,
        element: &ElementId,
        button: MouseButton,
        timeout_ms: u64,
    ) -> Result<()>;

    /// Insert text directly into the currently focused element (like `keyboard.insert_text`).
    async fn insert_text(&self, text: &str) -> Result<()>;

    /// Type text sequentially into `element` with per-character `delay_ms`.
    async fn type_text_sequentially(
        &self,
        element: &ElementId,
        text: &str,
        delay_ms: u64,
        timeout_ms: u64,
    ) -> Result<()>;

    /// Press a keyboard key or shortcut (e.g., `"Enter"`, `"Control+A"`, `"Backspace"`) globally.
    async fn press_key(&self, key: &str, timeout_ms: u64) -> Result<()>;

    /// Press a keyboard key or shortcut targeting `element`.
    async fn press_element_key(
        &self,
        element: &ElementId,
        key: &str,
        timeout_ms: u64,
    ) -> Result<()>;

    /// Attach local files to a file input element directly.
    async fn set_files(&self, element: &ElementId, paths: &[&Path]) -> Result<()>;

    /// Click an upload trigger that opens a native file chooser and supply `paths`.
    async fn choose_files_via_click(
        &self,
        trigger: &ElementId,
        paths: &[&Path],
        timeout_ms: u64,
    ) -> Result<()>;

    /// Dispatch a mouse wheel scroll event at the current pointer position.
    async fn wheel(&self, delta_x: f64, delta_y: f64) -> Result<()>;

    /// Begin capturing matching network requests and responses for a write operation.
    async fn start_network_capture(
        &self,
        filter: CaptureFilter,
    ) -> Result<Box<dyn NetworkCaptureHandle>>;
}

/// High-level browser session abstraction used by chat and video write flows.
#[async_trait]
pub trait UiSession: Send + Sync {
    /// Borrow the active [`UiPage`].
    fn page(&self) -> &dyn UiPage;

    /// Ensure the session is logged in and initialized before running a UI flow.
    async fn ensure_ready(&self) -> Result<()>;

    /// Navigate the session's active page to `url`.
    async fn open(&self, url: &str) -> Result<()>;

    /// Detect any active auth/verification challenge kind on the current page.
    async fn verification_kind(&self) -> Result<Option<String>> {
        crate::components::AuthChallengeComponent::new(self.page())
            .detect()
            .await
    }

    /// Return `true` when any auth/verification challenge is currently visible.
    async fn verification_visible(&self) -> Result<bool> {
        Ok(self.verification_kind().await?.is_some())
    }
}

// Compile-time checks guaranteeing object safety and Send + Sync bounds.
const _: fn(&dyn UiPage) = |_| {};
const _: fn(&dyn UiSession) = |_| {};
const _: fn(&dyn NetworkCaptureHandle) = |_| {};
