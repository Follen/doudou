//! Native async WebSocket Chrome `DevTools` Protocol (CDP) client with Patchright-inspired stealth.
//!
//! This transport **never** sends `Runtime.enable` or `Console.enable`. JavaScript evaluation
//! uses `Page.createIsolatedWorld` (`"doudou_isolated"`) or direct `Runtime.evaluate` with
//! `awaitPromise: true, returnByValue: true` without enabling runtime inspection domains.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use doudou_core::{DoudouError, Result, utc_now};
use futures_util::{SinkExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::{Notify, RwLock, broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, trace, warn};

use crate::chrome::ChromeProcess;

/// Isolated world name used for stealth DOM evaluation without calling `Runtime.enable`.
pub const ISOLATED_WORLD_NAME: &str = "doudou_isolated";

/// Default timeout for a single CDP JSON-RPC command round-trip.
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// CDP methods strictly forbidden by Patchright-style stealth rules.
pub const FORBIDDEN_CDP_METHODS: &[&str] = &["Runtime.enable", "Console.enable"];

/// URL patterns blocked when aborting media streams via `Network.setBlockedURLs`.
const MEDIA_ONLY_BLOCKED_URLS: &[&str] = &[
    "*.mp4*",
    "*.m3u8*",
    "*.m4s*",
    "*.flv*",
    "*.webm*",
    "*.ts?*",
    "*://*.douyinvod.com/*",
];

/// Extended URL patterns blocked in [`MediaBlockMode::HeavyAssets`].
const HEAVY_ASSET_BLOCKED_URLS: &[&str] = &[
    "*.mp4*",
    "*.m3u8*",
    "*.m4s*",
    "*.flv*",
    "*.webm*",
    "*.woff*",
    "*.ttf*",
    "*.otf*",
    "*.png*",
    "*.jpg*",
    "*.jpeg*",
    "*.webp*",
    "*.gif*",
    "*://*.douyinvod.com/*",
];

/// Raw CDP event emitted by the browser over the WebSocket stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CdpEvent {
    /// CDP event method name (e.g. `Page.loadEventFired`, `Network.responseReceived`).
    pub method: String,
    /// Event parameter payload.
    pub params: Value,
    /// Attached target session id when the event originates from a flattened session.
    pub session_id: Option<String>,
}

/// JavaScript execution world selector for [`CdpPageOps::eval_json`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalWorld {
    /// Page main world (shares cookies/origin/page globals; runs without `Runtime.enable`).
    Main,
    /// Dedicated isolated world (`"doudou_isolated"`) created via `Page.createIsolatedWorld`.
    Isolated,
}

/// Page navigation readiness target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadWait {
    /// Wait until `DOMContentLoaded` (`document.readyState` is `interactive` or `complete`).
    DomContentLoaded,
    /// Wait until full `load` (`document.readyState` is `complete`).
    Load,
}

/// Mouse button for synthetic input dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    /// Primary (left) mouse button.
    Left,
    /// Secondary (right) mouse button.
    Right,
    /// Middle mouse button.
    Middle,
}

impl MouseButton {
    fn as_cdp(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
        }
    }

    fn buttons_mask(self) -> u32 {
        match self {
            Self::Left => 1,
            Self::Right => 2,
            Self::Middle => 4,
        }
    }
}

/// Keyboard modifier flags for CDP `Input.dispatchKeyEvent`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyModifiers {
    /// Alt / Option modifier.
    pub alt: bool,
    /// Control modifier.
    pub ctrl: bool,
    /// Meta / Command / Windows modifier.
    pub meta: bool,
    /// Shift modifier.
    pub shift: bool,
}

impl KeyModifiers {
    /// Encode modifiers into the CDP bitmask (`Alt=1, Ctrl=2, Meta=4, Shift=8`).
    pub fn to_cdp_bitmask(self) -> u8 {
        let mut mask = 0u8;
        if self.alt {
            mask |= 1;
        }
        if self.ctrl {
            mask |= 2;
        }
        if self.meta {
            mask |= 4;
        }
        if self.shift {
            mask |= 8;
        }
        mask
    }
}

/// Specification for a synthetic keyboard key press (`keyDown` + `keyUp`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEventSpec {
    /// DOM `KeyboardEvent.key` value (e.g. `"Enter"`, `"Backspace"`, `"a"`).
    pub key: String,
    /// Optional DOM `KeyboardEvent.code` value (e.g. `"Enter"`, `"KeyA"`).
    pub code: Option<String>,
    /// Optional inserted character text for `keyDown`.
    pub text: Option<String>,
    /// Optional Windows virtual key code (e.g. `13` for Enter, `8` for Backspace).
    pub windows_virtual_key_code: Option<i32>,
    /// Active modifier keys.
    pub modifiers: KeyModifiers,
}

impl KeyEventSpec {
    /// Construct a simple key spec by DOM key name with common virtual key defaults.
    pub fn named(key: impl Into<String>) -> Self {
        let key = key.into();
        let (code, vk, text) = match key.as_str() {
            "Enter" => (Some("Enter".to_owned()), Some(13), Some("\r".to_owned())),
            "Backspace" => (Some("Backspace".to_owned()), Some(8), None),
            "Escape" => (Some("Escape".to_owned()), Some(27), None),
            "Tab" => (Some("Tab".to_owned()), Some(9), None),
            "ArrowDown" => (Some("ArrowDown".to_owned()), Some(40), None),
            "ArrowUp" => (Some("ArrowUp".to_owned()), Some(38), None),
            _ => (None, None, None),
        };
        Self {
            key,
            code,
            text,
            windows_virtual_key_code: vk,
            modifiers: KeyModifiers::default(),
        }
    }
}

/// Desired body encoding for [`CdpPageOps::fetch_in_page`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchResponseKind {
    /// Return UTF-8 `body_text` only.
    Text,
    /// Return base64-encoded `body_base64` only.
    Binary,
    /// Return both `body_text` (when text-like) and `body_base64`.
    Both,
}

/// Request specification for in-page `fetch()` execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchRequest {
    /// Target URL fetched from the active page's origin and cookie jar.
    pub url: String,
    /// HTTP method (e.g. `"GET"`, `"POST"`).
    pub method: String,
    /// Optional HTTP request headers.
    pub headers: BTreeMap<String, String>,
    /// Optional base64-encoded request body bytes.
    pub body_base64: Option<String>,
    /// Request timeout in milliseconds enforced via `AbortController`.
    pub timeout_ms: u64,
    /// Desired response body representation.
    pub response_kind: FetchResponseKind,
}

impl FetchRequest {
    /// Create a `GET` request returning UTF-8 text with a 15-second timeout.
    pub fn get_text(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: "GET".to_owned(),
            headers: BTreeMap::new(),
            body_base64: None,
            timeout_ms: 15_000,
            response_kind: FetchResponseKind::Text,
        }
    }

    /// Create a `POST` request with raw binary payload (e.g. `application/x-protobuf`).
    pub fn post_bytes(url: impl Into<String>, content_type: &str, body: &[u8]) -> Self {
        let mut headers = BTreeMap::new();
        headers.insert("content-type".to_owned(), content_type.to_owned());
        Self {
            url: url.into(),
            method: "POST".to_owned(),
            headers,
            body_base64: Some(BASE64.encode(body)),
            timeout_ms: 15_000,
            response_kind: FetchResponseKind::Binary,
        }
    }

    /// Override the request timeout in milliseconds.
    #[must_use]
    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Override the desired response body kind.
    #[must_use]
    pub fn with_response_kind(mut self, kind: FetchResponseKind) -> Self {
        self.response_kind = kind;
        self
    }
}

/// Result returned by in-page `fetch()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchResponse {
    /// HTTP status code.
    pub status: u16,
    /// True when status is in `200..300`.
    pub ok: bool,
    /// Lowercased response headers.
    pub headers: BTreeMap<String, String>,
    /// Response body as UTF-8 text when requested.
    pub body_text: Option<String>,
    /// Response body as base64 when requested.
    pub body_base64: Option<String>,
}

impl FetchResponse {
    /// Decode `body_base64` into raw bytes if present.
    pub fn decode_bytes(&self) -> Result<Vec<u8>> {
        let Some(encoded) = &self.body_base64 else {
            return Ok(self
                .body_text
                .as_ref()
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_default());
        };
        BASE64.decode(encoded).map_err(|err| {
            DoudouError::new(
                "PROTOCOL_DECODE_ERROR",
                format!("Failed to base64-decode in-page fetch body: {err}"),
            )
        })
    }

    /// Parse `body_text` as JSON.
    pub fn parse_json(&self) -> Result<Value> {
        let text = self.body_text.as_deref().ok_or_else(|| {
            DoudouError::new(
                "PROTOCOL_DECODE_ERROR",
                "In-page fetch response does not contain text body.",
            )
        })?;
        serde_json::from_str(text).map_err(|err| {
            DoudouError::new(
                "PROTOCOL_DECODE_ERROR",
                format!("Failed to parse in-page fetch response as JSON: {err}"),
            )
        })
    }
}

/// Browser cookie returned by CDP `Network.getCookies`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Cookie domain.
    pub domain: String,
    /// Cookie path.
    pub path: String,
    /// Expiration timestamp in Unix seconds (`-1` for session cookies).
    pub expires: f64,
    /// `HttpOnly` flag.
    pub http_only: bool,
    /// `Secure` flag.
    pub secure: bool,
    /// Optional `SameSite` attribute.
    pub same_site: Option<String>,
}

/// Rectangular viewport clip region passed to CDP `Page.captureScreenshot`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScreenshotClip {
    /// Left offset in CSS pixels.
    pub x: f64,
    /// Top offset in CSS pixels.
    pub y: f64,
    /// Width in CSS pixels.
    pub width: f64,
    /// Height in CSS pixels.
    pub height: f64,
    /// Device scale factor (e.g. `2.0` for crisp QR codes).
    pub scale: f64,
}

/// Media/asset blocking policy for [`CdpPageOps::enable_media_abort`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaBlockMode {
    /// Block heavy video/audio streams while keeping images and fonts intact.
    MediaOnly,
    /// Block video/audio streams, web fonts, and images for minimal bandwidth.
    HeavyAssets,
}

/// Captured network request/response pair collected via CDP `Network` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapturedNetworkExchange {
    /// CDP `requestId`.
    pub request_id: String,
    /// Full request URL.
    pub url: String,
    /// HTTP method.
    pub method: String,
    /// HTTP status code once `Network.responseReceived` arrives.
    pub status: Option<u16>,
    /// Response MIME type.
    pub mime_type: Option<String>,
    /// Request headers.
    pub request_headers: BTreeMap<String, String>,
    /// Response headers.
    pub response_headers: BTreeMap<String, String>,
    /// Captured request body bytes when present (e.g. `POST` Protobuf payload).
    pub request_body: Option<Vec<u8>>,
    /// Captured response body bytes fetched via `Network.getResponseBody`.
    pub response_body: Option<Vec<u8>>,
    /// Resource type reported by Chromium (e.g. `Fetch`, `XHR`, `Media`).
    pub resource_type: Option<String>,
    /// UTC timestamp when captured.
    pub timestamp: String,
}

/// Thread-safe buffer of captured network exchanges populated by `enable_network_capture`.
#[derive(Debug, Clone, Default)]
pub struct NetworkCaptureBuffer {
    inner: Arc<Mutex<Vec<CapturedNetworkExchange>>>,
    notify: Arc<Notify>,
}

impl NetworkCaptureBuffer {
    /// Create an empty capture buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record or update an exchange by `request_id` and wake waiters.
    pub fn upsert(&self, exchange: CapturedNetworkExchange) {
        {
            let mut guard = self.inner.lock().expect("network capture lock poisoned");
            if let Some(existing) = guard
                .iter_mut()
                .find(|item| item.request_id == exchange.request_id)
            {
                *existing = exchange;
            } else {
                guard.push(exchange);
            }
        }
        self.notify.notify_waiters();
    }

    /// Update an existing exchange in-place via a mutator closure.
    pub fn update_by_id(
        &self,
        request_id: &str,
        updater: impl FnOnce(&mut CapturedNetworkExchange),
    ) -> bool {
        let found_existing = {
            let mut guard = self.inner.lock().expect("network capture lock poisoned");
            if let Some(existing) = guard.iter_mut().find(|item| item.request_id == request_id) {
                updater(existing);
                true
            } else {
                false
            }
        };
        if found_existing {
            self.notify.notify_waiters();
        }
        found_existing
    }

    /// Snapshot all captured exchanges.
    pub fn snapshot(&self) -> Vec<CapturedNetworkExchange> {
        self.inner
            .lock()
            .expect("network capture lock poisoned")
            .clone()
    }

    /// Drain and return all captured exchanges.
    pub fn take_all(&self) -> Vec<CapturedNetworkExchange> {
        let mut guard = self.inner.lock().expect("network capture lock poisoned");
        std::mem::take(&mut *guard)
    }

    /// Clear all captured exchanges.
    pub fn clear(&self) {
        self.inner
            .lock()
            .expect("network capture lock poisoned")
            .clear();
    }

    /// Wait until any captured exchange satisfies `predicate`, or return `None` on timeout.
    pub async fn wait_for(
        &self,
        timeout: Duration,
        predicate: impl Fn(&CapturedNetworkExchange) -> bool,
    ) -> Option<CapturedNetworkExchange> {
        let deadline = Instant::now() + timeout;
        loop {
            let notified = self.notify.notified();
            {
                let guard = self.inner.lock().expect("network capture lock poisoned");
                if let Some(found) = guard.iter().find(|item| predicate(item)) {
                    return Some(found.clone());
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            if tokio::time::timeout(remaining, notified).await.is_err() {
                return None;
            }
        }
    }
}

/// Async abstraction implemented by both live [`CdpPage`] and [`crate::fake::FakeCdpPage`].
#[async_trait]
pub trait CdpPageOps: Send + Sync {
    /// Navigate the page to `url` and wait for `wait` within `timeout`.
    async fn goto(&self, url: &str, wait: LoadWait, timeout: Duration) -> Result<String>;

    /// Wait until the current page satisfies `wait` within `timeout`.
    async fn wait_for_load(&self, wait: LoadWait, timeout: Duration) -> Result<()>;

    /// Return the current top-level page URL.
    async fn current_url(&self) -> Result<String>;

    /// Return the current document title.
    async fn title(&self) -> Result<String>;

    /// Evaluate a JavaScript expression returning JSON in either [`EvalWorld::Main`] or [`EvalWorld::Isolated`].
    async fn eval_json(&self, expression: &str, world: EvalWorld) -> Result<Value>;

    /// Execute an in-page `fetch()` request using the page's origin and cookie jar.
    async fn fetch_in_page(&self, request: &FetchRequest) -> Result<FetchResponse>;

    /// Read cookies matching the given URLs (or all cookies when `urls` is empty).
    async fn cookies(&self, urls: &[&str]) -> Result<Vec<Cookie>>;

    /// Dispatch a synthetic mouse click at viewport coordinates `(x, y)`.
    async fn click_xy(&self, x: f64, y: f64, button: MouseButton, click_count: u32) -> Result<()>;

    /// Move the mouse cursor to viewport coordinates `(x, y)`.
    async fn mouse_move(&self, x: f64, y: f64) -> Result<()>;

    /// Dispatch a mouse wheel scroll event at `(x, y)` with `(delta_x, delta_y)`.
    async fn mouse_wheel(&self, x: f64, y: f64, delta_x: f64, delta_y: f64) -> Result<()>;

    /// Insert text directly into the focused input/editor via `Input.insertText`.
    async fn insert_text(&self, text: &str) -> Result<()>;

    /// Dispatch a keyboard `keyDown` + `keyUp` pair.
    async fn dispatch_key(&self, spec: &KeyEventSpec) -> Result<()>;

    /// Populate an `<input type="file">` matching `css_selector` with local file paths.
    async fn set_file_input_files(&self, css_selector: &str, files: &[PathBuf]) -> Result<()>;

    /// Capture a PNG screenshot of the current viewport.
    async fn screenshot_png(&self) -> Result<Vec<u8>>;

    /// Capture a PNG screenshot clipped to a specific viewport rectangle.
    async fn screenshot_clip_png(&self, clip: ScreenshotClip) -> Result<Vec<u8>> {
        let _ = clip;
        self.screenshot_png().await
    }

    /// Subscribe to network traffic matching any substring in `url_patterns` (all traffic if empty).
    async fn enable_network_capture(
        &self,
        url_patterns: Vec<String>,
    ) -> Result<NetworkCaptureBuffer>;

    /// Block heavy media/asset streams using `Network.setBlockedURLs` and/or `Fetch.enable`.
    async fn enable_media_abort(&self, mode: MediaBlockMode) -> Result<()>;

    /// Return the count of requests aborted by media blocking.
    fn blocked_media_requests(&self) -> usize;

    /// Access the active network capture buffer, if enabled.
    fn capture_buffer(&self) -> NetworkCaptureBuffer;

    /// Close this page target cleanly.
    async fn close(&self) -> Result<()>;
}

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>;

/// Multiplexed async WebSocket connection to Chrome `DevTools` Protocol.
#[derive(Debug)]
pub struct CdpConnection {
    ws_url: String,
    next_id: AtomicU64,
    tx: mpsc::Sender<String>,
    pending: PendingMap,
    events: broadcast::Sender<CdpEvent>,
    closed: Arc<AtomicBool>,
    reader_task: Mutex<Option<JoinHandle<()>>>,
    writer_task: Mutex<Option<JoinHandle<()>>>,
}

impl CdpConnection {
    /// Open a native async WebSocket connection to `ws_url`.
    pub async fn connect(ws_url: &str) -> Result<Self> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|err| {
                DoudouError::new(
                    "CDP_CONNECT_FAILED",
                    format!("Failed to connect CDP WebSocket {ws_url}: {err}"),
                )
            })?;

        let (mut ws_sink, mut ws_reader) = ws_stream.split();
        let (tx, mut rx) = mpsc::channel::<String>(1024);
        let (events, _) = broadcast::channel::<CdpEvent>(1024);
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));

        let writer_closed = Arc::clone(&closed);
        let writer_task = tokio::spawn(async move {
            while let Some(payload) = rx.recv().await {
                if let Err(err) = ws_sink.send(Message::text(payload)).await {
                    debug!(error = %err, "CDP WebSocket writer closed");
                    break;
                }
            }
            let _ = ws_sink.close().await;
            writer_closed.store(true, Ordering::SeqCst);
        });

        let reader_pending = Arc::clone(&pending);
        let reader_events = events.clone();
        let reader_closed = Arc::clone(&closed);
        let reader_task = tokio::spawn(async move {
            while let Some(frame) = ws_reader.next().await {
                let text = match frame {
                    Ok(Message::Text(text)) => text,
                    Ok(Message::Binary(bytes)) => match String::from_utf8(bytes.to_vec()) {
                        Ok(s) => s.into(),
                        Err(_) => continue,
                    },
                    Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => continue,
                    Ok(Message::Close(_)) => break,
                    Err(err) => {
                        debug!(error = %err, "CDP WebSocket reader error");
                        break;
                    }
                };

                let parsed: Value = match serde_json::from_str(text.as_str()) {
                    Ok(val) => val,
                    Err(err) => {
                        warn!(error = %err, "ignoring malformed CDP JSON frame");
                        continue;
                    }
                };

                if let Some(id) = parsed.get("id").and_then(Value::as_u64) {
                    let waiter = {
                        let mut map = reader_pending.lock().expect("pending CDP map poisoned");
                        map.remove(&id)
                    };
                    if let Some(sender) = waiter {
                        let outcome = if let Some(err_obj) = parsed.get("error") {
                            let message = err_obj
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown CDP protocol error");
                            let code = err_obj.get("code").and_then(Value::as_i64);
                            Err(DoudouError::new(
                                "CDP_COMMAND_ERROR",
                                format!("CDP error: {message}"),
                            )
                            .with_details(json!({
                                "cdp_code": code,
                                "cdp_error": err_obj,
                            })))
                        } else {
                            Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
                        };
                        let _ = sender.send(outcome);
                    }
                } else if let Some(method) = parsed.get("method").and_then(Value::as_str) {
                    let event = CdpEvent {
                        method: method.to_owned(),
                        params: parsed.get("params").cloned().unwrap_or_else(|| json!({})),
                        session_id: parsed
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    };
                    trace!(method = %event.method, "received CDP event");
                    let _ = reader_events.send(event);
                }
            }

            reader_closed.store(true, Ordering::SeqCst);
            let drained: Vec<oneshot::Sender<Result<Value>>> = {
                let mut map = reader_pending.lock().expect("pending CDP map poisoned");
                map.drain().map(|(_, tx)| tx).collect()
            };
            for waiter in drained {
                let _ = waiter.send(Err(DoudouError::new(
                    "CDP_DISCONNECTED",
                    "CDP WebSocket connection closed before receiving a response.",
                )));
            }
        });

        Ok(Self {
            ws_url: ws_url.to_owned(),
            next_id: AtomicU64::new(1),
            tx,
            pending,
            events,
            closed,
            reader_task: Mutex::new(Some(reader_task)),
            writer_task: Mutex::new(Some(writer_task)),
        })
    }

    /// Return the WebSocket URL this connection targets.
    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// Return whether the underlying WebSocket transport has closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Subscribe to the broadcast stream of CDP events.
    pub fn subscribe(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    /// Send a CDP JSON-RPC command with [`DEFAULT_COMMAND_TIMEOUT`].
    ///
    /// Rejects `Runtime.enable` and `Console.enable` unconditionally to preserve stealth.
    pub async fn send_command(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value> {
        self.send_command_with_timeout(method, params, session_id, DEFAULT_COMMAND_TIMEOUT)
            .await
    }

    /// Send a CDP JSON-RPC command with an explicit timeout.
    pub async fn send_command_with_timeout(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Value> {
        if FORBIDDEN_CDP_METHODS.contains(&method) {
            return Err(DoudouError::new(
                "CDP_STEALTH_VIOLATION",
                format!("CDP method '{method}' is forbidden under Patchright-style stealth mode."),
            ));
        }
        if self.is_closed() {
            return Err(DoudouError::new(
                "CDP_DISCONNECTED",
                "Cannot send CDP command on a closed WebSocket connection.",
            ));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut envelope = Map::new();
        envelope.insert("id".to_owned(), json!(id));
        envelope.insert("method".to_owned(), json!(method));
        if !params.is_null() {
            envelope.insert("params".to_owned(), params);
        }
        if let Some(sid) = session_id {
            envelope.insert("sessionId".to_owned(), json!(sid));
        }
        let serialized = serde_json::to_string(&Value::Object(envelope)).map_err(|err| {
            DoudouError::new(
                "JSON_SERIALIZE_ERROR",
                format!("Failed to serialize CDP command {method}: {err}"),
            )
        })?;

        let (resp_tx, resp_rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().expect("pending CDP map poisoned");
            map.insert(id, resp_tx);
        }

        if let Err(err) = self.tx.send(serialized).await {
            let mut map = self.pending.lock().expect("pending CDP map poisoned");
            map.remove(&id);
            return Err(DoudouError::new(
                "CDP_DISCONNECTED",
                format!("Failed queueing CDP command {method}: {err}"),
            ));
        }

        match tokio::time::timeout(timeout, resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(DoudouError::new(
                "CDP_DISCONNECTED",
                format!("CDP connection dropped while waiting for {method}."),
            )),
            Err(_) => {
                let mut map = self.pending.lock().expect("pending CDP map poisoned");
                map.remove(&id);
                Err(DoudouError::new(
                    "CDP_COMMAND_TIMEOUT",
                    format!("Timed out waiting for CDP command {method}."),
                )
                .with_details(json!({
                    "method": method,
                    "timeout_ms": timeout.as_millis(),
                })))
            }
        }
    }

    /// Close the WebSocket transport and abort background I/O tasks.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Some(handle) = self
            .reader_task
            .lock()
            .expect("reader task lock poisoned")
            .take()
        {
            handle.abort();
        }
        if let Some(handle) = self
            .writer_task
            .lock()
            .expect("writer task lock poisoned")
            .take()
        {
            handle.abort();
        }
    }
}

impl Drop for CdpConnection {
    fn drop(&mut self) {
        self.close();
    }
}

/// Owned browser handle pairing a [`ChromeProcess`] with its [`CdpConnection`].
///
/// Ensures clean process termination on `close()` and best-effort shutdown on `Drop`.
#[derive(Debug)]
pub struct CdpBrowser {
    process: ChromeProcess,
    connection: Arc<CdpConnection>,
}

impl CdpBrowser {
    /// Launch Chrome and immediately connect to its browser-level CDP WebSocket.
    pub async fn launch(options: crate::chrome::ChromeLaunchOptions) -> Result<Self> {
        let mut process = ChromeProcess::launch(options).await?;
        let ws_url = process.endpoint().ws_url.clone();
        let connection = match CdpConnection::connect(&ws_url).await {
            Ok(conn) => Arc::new(conn),
            Err(err) => {
                let _ = process.close().await;
                return Err(err);
            }
        };
        Ok(Self {
            process,
            connection,
        })
    }

    /// Access the underlying [`ChromeProcess`].
    pub fn process(&self) -> &ChromeProcess {
        &self.process
    }

    /// Access the shared [`CdpConnection`].
    pub fn connection(&self) -> &Arc<CdpConnection> {
        &self.connection
    }

    /// Attach to an existing page target or create a fresh `about:blank` page target.
    pub async fn open_page(&self) -> Result<CdpPage> {
        CdpPage::attach_or_create(Arc::clone(&self.connection)).await
    }

    /// Close the CDP connection and cleanly terminate the Chrome process.
    pub async fn close(&mut self) -> Result<()> {
        let _ = self
            .connection
            .send_command_with_timeout("Browser.close", json!({}), None, Duration::from_secs(3))
            .await;
        self.connection.close();
        self.process.close().await
    }
}

impl Drop for CdpBrowser {
    fn drop(&mut self) {
        self.connection.close();
    }
}

#[derive(Debug, Clone)]
enum CapturePatternState {
    Unconfigured,
    All,
    Filtered(Vec<String>),
}

/// Stealth CDP page session targeting a single browser tab without `Runtime.enable`.
#[derive(Debug)]
pub struct CdpPage {
    conn: Arc<CdpConnection>,
    target_id: String,
    session_id: String,
    main_frame_id: RwLock<String>,
    isolated_context_id: RwLock<Option<i64>>,
    capture_buffer: NetworkCaptureBuffer,
    capture_patterns: Arc<Mutex<CapturePatternState>>,
    capture_started: AtomicBool,
    blocked_media: Arc<AtomicUsize>,
    background_tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl CdpPage {
    /// Attach to an existing `"page"` target or create a new `about:blank` target.
    pub async fn attach_or_create(conn: Arc<CdpConnection>) -> Result<Self> {
        let targets_res = conn
            .send_command("Target.getTargets", json!({}), None)
            .await?;
        let existing_target = targets_res
            .get("targetInfos")
            .and_then(Value::as_array)
            .and_then(|infos| {
                infos.iter().find_map(|info| {
                    if info.get("type").and_then(Value::as_str) == Some("page") {
                        info.get("targetId")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    } else {
                        None
                    }
                })
            });

        let target_id = if let Some(tid) = existing_target {
            tid
        } else {
            let created = conn
                .send_command("Target.createTarget", json!({ "url": "about:blank" }), None)
                .await?;
            created
                .get("targetId")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    DoudouError::new(
                        "CDP_TARGET_ERROR",
                        "Target.createTarget response missing targetId.",
                    )
                })?
        };

        Self::attach(conn, target_id).await
    }

    /// Attach to a specific `target_id` with flattened CDP sessions.
    pub async fn attach(conn: Arc<CdpConnection>, target_id: String) -> Result<Self> {
        let attached = conn
            .send_command(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
                None,
            )
            .await?;
        let session_id = attached
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                DoudouError::new(
                    "CDP_TARGET_ERROR",
                    "Target.attachToTarget response missing sessionId.",
                )
            })?;

        // Enable Page events only; NEVER call Runtime.enable or Console.enable.
        conn.send_command("Page.enable", json!({}), Some(&session_id))
            .await?;

        // Best-effort stealth document bootstrap to ensure navigator.webdriver stays undefined.
        let _ = conn
            .send_command(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({
                    "source": "try { Object.defineProperty(navigator, 'webdriver', { get: () => undefined }); } catch (_) {}"
                }),
                Some(&session_id),
            )
            .await;

        // Best-effort headless User-Agent + Sec-CH-UA sanitization so Douyin endpoints never see
        // "HeadlessChrome" in HTTP headers or navigator.userAgentData.
        if let Ok(ver) = conn
            .send_command_with_timeout(
                "Browser.getVersion",
                json!({}),
                None,
                Duration::from_secs(2),
            )
            .await
            && let Some(raw_ua) = ver.get("userAgent").and_then(Value::as_str)
            && raw_ua.contains("HeadlessChrome/")
        {
            let clean_ua = raw_ua.replace("HeadlessChrome/", "Chrome/");
            let full_ver = ver
                .get("product")
                .and_then(Value::as_str)
                .and_then(|p| p.split('/').nth(1))
                .unwrap_or("131.0.0.0");
            let major = full_ver.split('.').next().unwrap_or("131");
            let (platform, platform_version) = if cfg!(windows) {
                ("Windows", "15.0.0")
            } else if cfg!(target_os = "macos") {
                ("macOS", "14.0.0")
            } else {
                ("Linux", "6.0.0")
            };
            let _ = conn
                .send_command_with_timeout(
                    "Emulation.setUserAgentOverride",
                    json!({
                        "userAgent": clean_ua,
                        "acceptLanguage": "zh-CN,zh;q=0.9",
                        "userAgentMetadata": {
                            "brands": [
                                { "brand": "Google Chrome", "version": major },
                                { "brand": "Chromium", "version": major },
                                { "brand": "Not_A Brand", "version": "24" }
                            ],
                            "fullVersionList": [
                                { "brand": "Google Chrome", "version": full_ver },
                                { "brand": "Chromium", "version": full_ver },
                                { "brand": "Not_A Brand", "version": "24.0.0.0" }
                            ],
                            "fullVersion": full_ver,
                            "platform": platform,
                            "platformVersion": platform_version,
                            "architecture": if cfg!(target_arch = "aarch64") { "arm" } else { "x86" },
                            "model": "",
                            "mobile": false
                        }
                    }),
                    Some(&session_id),
                    Duration::from_secs(2),
                )
                .await;
        }

        let frame_tree = conn
            .send_command("Page.getFrameTree", json!({}), Some(&session_id))
            .await?;
        let main_frame_id = frame_tree
            .pointer("/frameTree/frame/id")
            .and_then(Value::as_str)
            .unwrap_or(&target_id)
            .to_owned();

        Ok(Self {
            conn,
            target_id,
            session_id,
            main_frame_id: RwLock::new(main_frame_id),
            isolated_context_id: RwLock::new(None),
            capture_buffer: NetworkCaptureBuffer::new(),
            capture_patterns: Arc::new(Mutex::new(CapturePatternState::Unconfigured)),
            capture_started: AtomicBool::new(false),
            blocked_media: Arc::new(AtomicUsize::new(0)),
            background_tasks: Mutex::new(Vec::new()),
        })
    }

    /// Return the CDP target id.
    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    /// Return the flattened CDP session id.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Send a raw CDP command scoped to this page's flattened `sessionId`.
    pub async fn send(&self, method: &str, params: Value) -> Result<Value> {
        self.conn
            .send_command(method, params, Some(&self.session_id))
            .await
    }

    async fn ensure_isolated_world(&self) -> Result<i64> {
        if let Some(existing) = *self.isolated_context_id.read().await {
            return Ok(existing);
        }
        let frame_id = self.main_frame_id.read().await.clone();
        let response = self
            .send(
                "Page.createIsolatedWorld",
                json!({
                    "frameId": frame_id,
                    "worldName": ISOLATED_WORLD_NAME,
                    "grantUniveralAccess": true,
                }),
            )
            .await?;
        let context_id = response
            .get("executionContextId")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                DoudouError::new(
                    "CDP_EVAL_ERROR",
                    "Page.createIsolatedWorld response missing executionContextId.",
                )
            })?;
        *self.isolated_context_id.write().await = Some(context_id);
        Ok(context_id)
    }

    async fn invalidate_isolated_world(&self) {
        *self.isolated_context_id.write().await = None;
    }

    fn push_background_task(&self, handle: JoinHandle<()>) {
        self.background_tasks
            .lock()
            .expect("background tasks lock poisoned")
            .push(handle);
    }

    async fn capture_screenshot_inner(&self, clip: Option<ScreenshotClip>) -> Result<Vec<u8>> {
        let mut params = json!({ "format": "png" });
        if let Some(region) = clip {
            params["clip"] = json!({
                "x": region.x,
                "y": region.y,
                "width": region.width,
                "height": region.height,
                "scale": region.scale,
            });
        }
        let res = self.send("Page.captureScreenshot", params).await?;
        let encoded = res.get("data").and_then(Value::as_str).ok_or_else(|| {
            DoudouError::new(
                "CDP_SCREENSHOT_ERROR",
                "Page.captureScreenshot missing data.",
            )
        })?;
        BASE64.decode(encoded).map_err(|err| {
            DoudouError::new(
                "CDP_SCREENSHOT_ERROR",
                format!("Failed decoding screenshot PNG base64: {err}"),
            )
        })
    }

    fn abort_background_tasks(&self) {
        let tasks = {
            let mut guard = self
                .background_tasks
                .lock()
                .expect("background tasks lock poisoned");
            std::mem::take(&mut *guard)
        };
        for task in tasks {
            task.abort();
        }
    }
}

impl Drop for CdpPage {
    fn drop(&mut self) {
        self.abort_background_tasks();
    }
}

#[async_trait]
impl CdpPageOps for CdpPage {
    async fn goto(&self, url: &str, wait: LoadWait, timeout: Duration) -> Result<String> {
        let mut events = self.conn.subscribe();
        self.invalidate_isolated_world().await;

        let nav = self
            .conn
            .send_command_with_timeout(
                "Page.navigate",
                json!({ "url": url }),
                Some(&self.session_id),
                timeout,
            )
            .await?;
        if let Some(error_text) = nav.get("errorText").and_then(Value::as_str)
            && !error_text.is_empty()
        {
            return Err(DoudouError::new(
                "CDP_NAVIGATION_ERROR",
                format!("Navigation to {url} failed: {error_text}"),
            )
            .with_details(json!({ "url": url, "error_text": error_text })));
        }
        if let Some(frame_id) = nav.get("frameId").and_then(Value::as_str) {
            *self.main_frame_id.write().await = frame_id.to_owned();
        }

        let target_event = match wait {
            LoadWait::DomContentLoaded => "Page.domContentEventFired",
            LoadWait::Load => "Page.loadEventFired",
        };
        let deadline = Instant::now() + timeout;
        let mut saw_event = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining.min(Duration::from_millis(250)), events.recv())
                .await
            {
                Ok(Ok(ev))
                    if ev.session_id.as_deref() == Some(self.session_id.as_str())
                        && ev.method == target_event =>
                {
                    saw_event = true;
                    break;
                }
                Ok(Ok(_) | Err(broadcast::error::RecvError::Lagged(_))) => {}
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Err(_) => {
                    if self.is_ready_state_satisfied(wait).await {
                        saw_event = true;
                        break;
                    }
                }
            }
        }
        if !saw_event && !self.is_ready_state_satisfied(wait).await {
            return Err(DoudouError::new(
                "CDP_NAVIGATION_TIMEOUT",
                format!("Timed out waiting for {target_event} while navigating to {url}."),
            ));
        }

        self.invalidate_isolated_world().await;
        self.current_url().await
    }

    async fn wait_for_load(&self, wait: LoadWait, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.is_ready_state_satisfied(wait).await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(DoudouError::new(
            "CDP_NAVIGATION_TIMEOUT",
            "Timed out waiting for page readyState.",
        ))
    }

    async fn current_url(&self) -> Result<String> {
        let val = self
            .eval_json("window.location.href", EvalWorld::Main)
            .await?;
        val.as_str().map(str::to_owned).ok_or_else(|| {
            DoudouError::new("CDP_EVAL_ERROR", "window.location.href was not a string.")
        })
    }

    async fn title(&self) -> Result<String> {
        let val = self.eval_json("document.title", EvalWorld::Main).await?;
        Ok(val.as_str().unwrap_or_default().to_owned())
    }

    async fn eval_json(&self, expression: &str, world: EvalWorld) -> Result<Value> {
        let mut params = Map::new();
        params.insert("expression".to_owned(), json!(expression));
        params.insert("awaitPromise".to_owned(), json!(true));
        params.insert("returnByValue".to_owned(), json!(true));

        let result = match world {
            EvalWorld::Main => self.send("Runtime.evaluate", Value::Object(params)).await?,
            EvalWorld::Isolated => {
                let ctx = self.ensure_isolated_world().await?;
                params.insert("contextId".to_owned(), json!(ctx));
                match self
                    .send("Runtime.evaluate", Value::Object(params.clone()))
                    .await
                {
                    Ok(res) => res,
                    Err(err) => {
                        // Isolated world contexts reset across navigations; recreate once and retry.
                        self.invalidate_isolated_world().await;
                        if err.code == "CDP_COMMAND_ERROR" {
                            let fresh_ctx = self.ensure_isolated_world().await?;
                            params.insert("contextId".to_owned(), json!(fresh_ctx));
                            self.send("Runtime.evaluate", Value::Object(params)).await?
                        } else {
                            return Err(err);
                        }
                    }
                }
            }
        };

        if let Some(exception) = result.get("exceptionDetails") {
            let text = exception
                .pointer("/exception/description")
                .or_else(|| exception.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("JavaScript evaluation threw an exception");
            return Err(
                DoudouError::new("CDP_EVAL_ERROR", format!("JavaScript error: {text}"))
                    .with_details(exception.clone()),
            );
        }

        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn fetch_in_page(&self, request: &FetchRequest) -> Result<FetchResponse> {
        let payload = serde_json::to_string(&json!({
            "url": request.url,
            "method": request.method,
            "headers": request.headers,
            "bodyBase64": request.body_base64,
            "timeoutMs": request.timeout_ms,
            "wantText": matches!(request.response_kind, FetchResponseKind::Text | FetchResponseKind::Both),
            "wantBase64": matches!(request.response_kind, FetchResponseKind::Binary | FetchResponseKind::Both),
        }))
        .map_err(|err| {
            DoudouError::new(
                "JSON_SERIALIZE_ERROR",
                format!("Failed serializing fetch request: {err}"),
            )
        })?;

        let script = format!(
            r"(async () => {{
  const req = {payload};
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), req.timeoutMs);
  try {{
    const init = {{
      method: req.method,
      credentials: 'include',
      headers: req.headers || {{}},
      signal: controller.signal,
    }};
    if (req.bodyBase64 !== null && req.bodyBase64 !== undefined) {{
      const raw = atob(req.bodyBase64);
      const bytes = new Uint8Array(raw.length);
      for (let i = 0; i < raw.length; i += 1) bytes[i] = raw.charCodeAt(i);
      init.body = bytes;
    }}
    const resp = await fetch(req.url, init);
    const headers = {{}};
    resp.headers.forEach((value, key) => {{ headers[key.toLowerCase()] = value; }});
    const buf = new Uint8Array(await resp.arrayBuffer());
    let body_text = null;
    let body_base64 = null;
    if (req.wantText) {{
      body_text = new TextDecoder('utf-8').decode(buf);
    }}
    if (req.wantBase64) {{
      let binary = '';
      const chunk = 0x8000;
      for (let i = 0; i < buf.length; i += chunk) {{
        binary += String.fromCharCode(...buf.subarray(i, i + chunk));
      }}
      body_base64 = btoa(binary);
    }}
    return {{
      status: resp.status,
      ok: resp.ok,
      headers,
      body_text,
      body_base64,
    }};
  }} finally {{
    clearTimeout(timer);
  }}
}})()"
        );

        let value = self.eval_json(&script, EvalWorld::Main).await?;
        serde_json::from_value(value).map_err(|err| {
            DoudouError::new(
                "CDP_EVAL_ERROR",
                format!("Invalid in-page fetch response structure: {err}"),
            )
        })
    }

    async fn cookies(&self, urls: &[&str]) -> Result<Vec<Cookie>> {
        let params = if urls.is_empty() {
            json!({})
        } else {
            json!({ "urls": urls })
        };
        let res = self.send("Network.getCookies", params).await?;
        let raw_list = res
            .get("cookies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut cookies = Vec::with_capacity(raw_list.len());
        for item in raw_list {
            cookies.push(Cookie {
                name: item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                value: item
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                domain: item
                    .get("domain")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                path: item
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("/")
                    .to_owned(),
                expires: item.get("expires").and_then(Value::as_f64).unwrap_or(-1.0),
                http_only: item
                    .get("httpOnly")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                secure: item.get("secure").and_then(Value::as_bool).unwrap_or(false),
                same_site: item
                    .get("sameSite")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
        Ok(cookies)
    }

    async fn click_xy(&self, x: f64, y: f64, button: MouseButton, click_count: u32) -> Result<()> {
        self.mouse_move(x, y).await?;
        let btn = button.as_cdp();
        let mask = button.buttons_mask();
        self.send(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mousePressed",
                "x": x,
                "y": y,
                "button": btn,
                "buttons": mask,
                "clickCount": click_count,
            }),
        )
        .await?;
        self.send(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseReleased",
                "x": x,
                "y": y,
                "button": btn,
                "buttons": 0,
                "clickCount": click_count,
            }),
        )
        .await?;
        Ok(())
    }

    async fn mouse_move(&self, x: f64, y: f64) -> Result<()> {
        self.send(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseMoved",
                "x": x,
                "y": y,
            }),
        )
        .await?;
        Ok(())
    }

    async fn mouse_wheel(&self, x: f64, y: f64, delta_x: f64, delta_y: f64) -> Result<()> {
        self.send(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel",
                "x": x,
                "y": y,
                "deltaX": delta_x,
                "deltaY": delta_y,
            }),
        )
        .await?;
        Ok(())
    }

    async fn insert_text(&self, text: &str) -> Result<()> {
        self.send("Input.insertText", json!({ "text": text }))
            .await?;
        Ok(())
    }

    async fn dispatch_key(&self, spec: &KeyEventSpec) -> Result<()> {
        let modifiers = spec.modifiers.to_cdp_bitmask();
        let mut down = Map::new();
        down.insert("type".to_owned(), json!("keyDown"));
        down.insert("key".to_owned(), json!(spec.key));
        down.insert("modifiers".to_owned(), json!(modifiers));
        if let Some(code) = &spec.code {
            down.insert("code".to_owned(), json!(code));
        }
        if let Some(text) = &spec.text {
            down.insert("text".to_owned(), json!(text));
            down.insert("unmodifiedText".to_owned(), json!(text));
        }
        if let Some(vk) = spec.windows_virtual_key_code {
            down.insert("windowsVirtualKeyCode".to_owned(), json!(vk));
            down.insert("nativeVirtualKeyCode".to_owned(), json!(vk));
        }
        self.send("Input.dispatchKeyEvent", Value::Object(down))
            .await?;

        let mut up = Map::new();
        up.insert("type".to_owned(), json!("keyUp"));
        up.insert("key".to_owned(), json!(spec.key));
        up.insert("modifiers".to_owned(), json!(modifiers));
        if let Some(code) = &spec.code {
            up.insert("code".to_owned(), json!(code));
        }
        if let Some(vk) = spec.windows_virtual_key_code {
            up.insert("windowsVirtualKeyCode".to_owned(), json!(vk));
            up.insert("nativeVirtualKeyCode".to_owned(), json!(vk));
        }
        self.send("Input.dispatchKeyEvent", Value::Object(up))
            .await?;
        Ok(())
    }

    async fn set_file_input_files(&self, css_selector: &str, files: &[PathBuf]) -> Result<()> {
        let doc = self.send("DOM.getDocument", json!({ "depth": 1 })).await?;
        let root_id = doc
            .pointer("/root/nodeId")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                DoudouError::new("CDP_DOM_ERROR", "DOM.getDocument missing root nodeId.")
            })?;
        let node = self
            .send(
                "DOM.querySelector",
                json!({ "nodeId": root_id, "selector": css_selector }),
            )
            .await?;
        let node_id = node
            .get("nodeId")
            .and_then(Value::as_i64)
            .filter(|id| *id > 0)
            .ok_or_else(|| {
                DoudouError::new(
                    "CDP_DOM_ERROR",
                    format!("No DOM node matched selector: {css_selector}"),
                )
            })?;
        let file_strings: Vec<String> = files.iter().map(|p| p.display().to_string()).collect();
        self.send(
            "DOM.setFileInputFiles",
            json!({ "nodeId": node_id, "files": file_strings }),
        )
        .await?;
        Ok(())
    }

    async fn screenshot_png(&self) -> Result<Vec<u8>> {
        self.capture_screenshot_inner(None).await
    }

    async fn screenshot_clip_png(&self, clip: ScreenshotClip) -> Result<Vec<u8>> {
        self.capture_screenshot_inner(Some(clip)).await
    }

    async fn enable_network_capture(
        &self,
        url_patterns: Vec<String>,
    ) -> Result<NetworkCaptureBuffer> {
        {
            let mut guard = self
                .capture_patterns
                .lock()
                .expect("capture patterns lock poisoned");
            if url_patterns.is_empty() {
                *guard = CapturePatternState::All;
            } else {
                match &mut *guard {
                    CapturePatternState::Unconfigured => {
                        *guard = CapturePatternState::Filtered(url_patterns);
                    }
                    CapturePatternState::All => {}
                    CapturePatternState::Filtered(existing) => {
                        for pattern in url_patterns {
                            if !existing.contains(&pattern) {
                                existing.push(pattern);
                            }
                        }
                    }
                }
            }
        }

        if self.capture_started.swap(true, Ordering::SeqCst) {
            return Ok(self.capture_buffer.clone());
        }

        let mut events = self.conn.subscribe();
        if let Err(err) = self.send("Network.enable", json!({})).await {
            self.capture_started.store(false, Ordering::SeqCst);
            return Err(err);
        }

        let conn = Arc::clone(&self.conn);
        let session_id = self.session_id.clone();
        let buffer = self.capture_buffer.clone();
        let patterns = Arc::clone(&self.capture_patterns);

        let task = tokio::spawn(async move {
            let mut tracked: HashMap<String, String> = HashMap::new();
            loop {
                let event = match events.recv().await {
                    Ok(ev) => ev,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if event.session_id.as_deref() != Some(session_id.as_str()) {
                    continue;
                }
                match event.method.as_str() {
                    "Network.requestWillBeSent" => {
                        let Some(req_id) = event
                            .params
                            .get("requestId")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                        else {
                            continue;
                        };
                        let url = event
                            .params
                            .pointer("/request/url")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        if !matches_capture_state(&url, &patterns) {
                            continue;
                        }
                        let method = event
                            .params
                            .pointer("/request/method")
                            .and_then(Value::as_str)
                            .unwrap_or("GET")
                            .to_owned();
                        let request_headers =
                            extract_headers(event.params.pointer("/request/headers"));
                        let request_body = extract_post_data(event.params.pointer("/request"));
                        let resource_type = event
                            .params
                            .get("type")
                            .and_then(Value::as_str)
                            .map(str::to_owned);

                        tracked.insert(req_id.clone(), url.clone());
                        buffer.upsert(CapturedNetworkExchange {
                            request_id: req_id,
                            url,
                            method,
                            status: None,
                            mime_type: None,
                            request_headers,
                            response_headers: BTreeMap::new(),
                            request_body,
                            response_body: None,
                            resource_type,
                            timestamp: utc_now(),
                        });
                    }
                    "Network.responseReceived" => {
                        let Some(req_id) = event.params.get("requestId").and_then(Value::as_str)
                        else {
                            continue;
                        };
                        let url = event
                            .params
                            .pointer("/response/url")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        if !tracked.contains_key(req_id) && !matches_capture_state(&url, &patterns)
                        {
                            continue;
                        }
                        tracked
                            .entry(req_id.to_owned())
                            .or_insert_with(|| url.clone());
                        let status = event
                            .params
                            .pointer("/response/status")
                            .and_then(Value::as_u64)
                            .map(|s| s as u16);
                        let mime_type = event
                            .params
                            .pointer("/response/mimeType")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        let response_headers =
                            extract_headers(event.params.pointer("/response/headers"));
                        let resource_type = event
                            .params
                            .get("type")
                            .and_then(Value::as_str)
                            .map(str::to_owned);

                        let updated = buffer.update_by_id(req_id, |item| {
                            item.status = status;
                            item.mime_type.clone_from(&mime_type);
                            item.response_headers.clone_from(&response_headers);
                            if item.resource_type.is_none() {
                                item.resource_type.clone_from(&resource_type);
                            }
                        });
                        if !updated {
                            buffer.upsert(CapturedNetworkExchange {
                                request_id: req_id.to_owned(),
                                url,
                                method: "GET".to_owned(),
                                status,
                                mime_type,
                                request_headers: BTreeMap::new(),
                                response_headers,
                                request_body: None,
                                response_body: None,
                                resource_type,
                                timestamp: utc_now(),
                            });
                        }
                    }
                    "Network.loadingFinished" => {
                        let Some(req_id) = event
                            .params
                            .get("requestId")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                        else {
                            continue;
                        };
                        if !tracked.contains_key(&req_id) {
                            continue;
                        }
                        let conn_clone = Arc::clone(&conn);
                        let sid = session_id.clone();
                        let buf = buffer.clone();
                        tokio::spawn(async move {
                            let bytes = match conn_clone
                                .send_command_with_timeout(
                                    "Network.getResponseBody",
                                    json!({ "requestId": req_id }),
                                    Some(&sid),
                                    Duration::from_secs(5),
                                )
                                .await
                            {
                                Ok(body_res) => {
                                    if let Some(raw_body) =
                                        body_res.get("body").and_then(Value::as_str)
                                    {
                                        let base64_encoded = body_res
                                            .get("base64Encoded")
                                            .and_then(Value::as_bool)
                                            .unwrap_or(false);
                                        if base64_encoded {
                                            BASE64.decode(raw_body).unwrap_or_default()
                                        } else {
                                            raw_body.as_bytes().to_vec()
                                        }
                                    } else {
                                        Vec::new()
                                    }
                                }
                                Err(err) => {
                                    debug!(
                                        request_id = %req_id,
                                        error = %err,
                                        "Network.getResponseBody failed after loadingFinished"
                                    );
                                    Vec::new()
                                }
                            };
                            buf.update_by_id(&req_id, |item| {
                                item.response_body = Some(bytes);
                            });
                        });
                    }
                    _ => {}
                }
            }
        });
        self.push_background_task(task);

        Ok(self.capture_buffer.clone())
    }

    async fn enable_media_abort(&self, mode: MediaBlockMode) -> Result<()> {
        let mut events = self.conn.subscribe();
        self.send("Network.enable", json!({})).await?;
        let urls = match mode {
            MediaBlockMode::MediaOnly => MEDIA_ONLY_BLOCKED_URLS,
            MediaBlockMode::HeavyAssets => HEAVY_ASSET_BLOCKED_URLS,
        };
        self.send("Network.setBlockedURLs", json!({ "urls": urls }))
            .await?;

        let patterns = match mode {
            MediaBlockMode::MediaOnly => vec![json!({ "resourceType": "Media" })],
            MediaBlockMode::HeavyAssets => vec![
                json!({ "resourceType": "Media" }),
                json!({ "resourceType": "Font" }),
                json!({ "resourceType": "Image" }),
            ],
        };
        self.send("Fetch.enable", json!({ "patterns": patterns }))
            .await?;

        let conn = Arc::clone(&self.conn);
        let session_id = self.session_id.clone();
        let blocked = Arc::clone(&self.blocked_media);

        let task = tokio::spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(ev) => ev,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if event.session_id.as_deref() != Some(session_id.as_str())
                    || event.method != "Fetch.requestPaused"
                {
                    continue;
                }
                let Some(request_id) = event
                    .params
                    .get("requestId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                else {
                    continue;
                };
                let resource_type = event
                    .params
                    .get("resourceType")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let should_block = match mode {
                    MediaBlockMode::MediaOnly => resource_type == "Media",
                    MediaBlockMode::HeavyAssets => {
                        matches!(resource_type, "Media" | "Font" | "Image")
                    }
                };
                let conn_clone = Arc::clone(&conn);
                let sid = session_id.clone();
                if should_block {
                    blocked.fetch_add(1, Ordering::Relaxed);
                    tokio::spawn(async move {
                        let _ = conn_clone
                            .send_command_with_timeout(
                                "Fetch.failRequest",
                                json!({
                                    "requestId": request_id,
                                    "errorReason": "BlockedByClient",
                                }),
                                Some(&sid),
                                Duration::from_secs(3),
                            )
                            .await;
                    });
                } else {
                    tokio::spawn(async move {
                        let _ = conn_clone
                            .send_command_with_timeout(
                                "Fetch.continueRequest",
                                json!({ "requestId": request_id }),
                                Some(&sid),
                                Duration::from_secs(3),
                            )
                            .await;
                    });
                }
            }
        });
        self.push_background_task(task);
        Ok(())
    }

    fn blocked_media_requests(&self) -> usize {
        self.blocked_media.load(Ordering::Relaxed)
    }

    fn capture_buffer(&self) -> NetworkCaptureBuffer {
        self.capture_buffer.clone()
    }

    async fn close(&self) -> Result<()> {
        self.abort_background_tasks();
        let _ = self
            .conn
            .send_command_with_timeout(
                "Target.closeTarget",
                json!({ "targetId": self.target_id }),
                None,
                Duration::from_secs(3),
            )
            .await;
        Ok(())
    }
}

impl CdpPage {
    async fn is_ready_state_satisfied(&self, wait: LoadWait) -> bool {
        let Ok(val) = self.eval_json("document.readyState", EvalWorld::Main).await else {
            return false;
        };
        matches!(
            (wait, val.as_str()),
            (LoadWait::DomContentLoaded, Some("interactive" | "complete"))
                | (LoadWait::Load, Some("complete"))
        )
    }
}

fn matches_patterns(url: &str, patterns: &[String]) -> bool {
    patterns.is_empty() || patterns.iter().any(|pattern| url.contains(pattern))
}

fn matches_capture_state(url: &str, state: &Mutex<CapturePatternState>) -> bool {
    let guard = state.lock().expect("capture patterns lock poisoned");
    match &*guard {
        CapturePatternState::Unconfigured => false,
        CapturePatternState::All => true,
        CapturePatternState::Filtered(patterns) => matches_patterns(url, patterns),
    }
}

fn extract_post_data(request: Option<&Value>) -> Option<Vec<u8>> {
    let req = request?;
    if let Some(entries) = req.get("postDataEntries").and_then(Value::as_array) {
        let mut combined = Vec::new();
        let mut decoded_any = false;
        for entry in entries {
            if let Some(b64) = entry.get("bytes").and_then(Value::as_str)
                && let Ok(chunk) = BASE64.decode(b64)
            {
                combined.extend_from_slice(&chunk);
                decoded_any = true;
            }
        }
        if decoded_any {
            return Some(combined);
        }
    }
    req.get("postData")
        .and_then(Value::as_str)
        .map(|s| s.as_bytes().to_vec())
}

fn extract_headers(value: Option<&Value>) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Some(obj) = value.and_then(Value::as_object) else {
        return map;
    };
    for (key, val) in obj {
        if let Some(s) = val.as_str() {
            map.insert(key.to_ascii_lowercase(), s.to_owned());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::*;

    /// Spawn a local mock CDP WebSocket server that records every received method and responds
    /// to standard Page/Target/Runtime/Network/Input/DOM/Fetch commands.
    async fn spawn_mock_cdp_server() -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local mock CDP server");
        let addr = listener.local_addr().expect("local addr");
        let recorded_methods = Arc::new(Mutex::new(Vec::<String>::new()));
        let recorded_clone = Arc::clone(&recorded_methods);

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept ws tcp");
            let ws = tokio_tungstenite::accept_async(stream)
                .await
                .expect("accept ws handshake");
            let (mut sink, mut reader) = ws.split();

            while let Some(Ok(msg)) = reader.next().await {
                let Message::Text(text) = msg else {
                    continue;
                };
                let req: Value = serde_json::from_str(text.as_str()).expect("valid json rpc");
                let id = req["id"].as_u64().expect("id");
                let method = req["method"].as_str().expect("method").to_owned();
                let session_id = req
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                recorded_clone
                    .lock()
                    .expect("recorded methods lock")
                    .push(method.clone());

                let result = match method.as_str() {
                    "Target.getTargets" => json!({
                        "targetInfos": [{ "targetId": "target-1", "type": "page" }]
                    }),
                    "Target.attachToTarget" => json!({ "sessionId": "session-1" }),
                    "Page.getFrameTree" => json!({
                        "frameTree": { "frame": { "id": "frame-main" } }
                    }),
                    "Page.createIsolatedWorld" => json!({ "executionContextId": 42 }),
                    "Page.navigate" => {
                        let url = req["params"]["url"].as_str().unwrap_or("about:blank");
                        // Emit Page.domContentEventFired right after navigation reply.
                        let reply = json!({
                            "id": id,
                            "result": { "frameId": "frame-main" }
                        });
                        sink.send(Message::text(reply.to_string())).await.unwrap();
                        let ev = json!({
                            "method": "Page.domContentEventFired",
                            "sessionId": session_id,
                            "params": { "timestamp": 1.0, "url": url }
                        });
                        sink.send(Message::text(ev.to_string())).await.unwrap();
                        continue;
                    }
                    "Runtime.evaluate" => {
                        let expr = req["params"]["expression"].as_str().unwrap_or("");
                        let ctx_id = req["params"].get("contextId").and_then(Value::as_i64);
                        if expr == "window.location.href" {
                            json!({ "result": { "type": "string", "value": "https://www.douyin.com/chat" } })
                        } else if expr == "document.readyState" {
                            json!({ "result": { "type": "string", "value": "complete" } })
                        } else if expr == "1 + 2" {
                            json!({ "result": { "type": "number", "value": 3 } })
                        } else if expr == "isolated_probe()" {
                            assert_eq!(ctx_id, Some(42), "isolated eval must pass contextId");
                            json!({ "result": { "type": "object", "value": { "world": "isolated" } } })
                        } else if expr.contains("AbortController") {
                            json!({
                                "result": {
                                    "type": "object",
                                    "value": {
                                        "status": 200,
                                        "ok": true,
                                        "headers": { "content-type": "application/json" },
                                        "body_text": "{\"status_code\":0}",
                                        "body_base64": null
                                    }
                                }
                            })
                        } else {
                            json!({ "result": { "type": "string", "value": "ok" } })
                        }
                    }
                    "Network.enable" => {
                        let reply = json!({ "id": id, "result": {} });
                        sink.send(Message::text(reply.to_string())).await.unwrap();
                        // Push synthetic Network events to verify capture + getResponseBody.
                        let req_ev = json!({
                            "method": "Network.requestWillBeSent",
                            "sessionId": session_id,
                            "params": {
                                "requestId": "req-100",
                                "type": "Fetch",
                                "request": {
                                    "url": "https://imapi.douyin.com/v1/message/get_message_by_init",
                                    "method": "POST",
                                    "headers": { "Content-Type": "application/x-protobuf" },
                                    "postData": "proto-req"
                                }
                            }
                        });
                        let resp_ev = json!({
                            "method": "Network.responseReceived",
                            "sessionId": session_id,
                            "params": {
                                "requestId": "req-100",
                                "type": "Fetch",
                                "response": {
                                    "url": "https://imapi.douyin.com/v1/message/get_message_by_init",
                                    "status": 200,
                                    "mimeType": "application/x-protobuf",
                                    "headers": { "X-Test": "1" }
                                }
                            }
                        });
                        let done_ev = json!({
                            "method": "Network.loadingFinished",
                            "sessionId": session_id,
                            "params": { "requestId": "req-100" }
                        });
                        sink.send(Message::text(req_ev.to_string())).await.unwrap();
                        sink.send(Message::text(resp_ev.to_string())).await.unwrap();
                        sink.send(Message::text(done_ev.to_string())).await.unwrap();
                        continue;
                    }
                    "Network.getResponseBody" => json!({
                        "body": BASE64.encode(b"\x08\x01\x12\x04test"),
                        "base64Encoded": true
                    }),
                    "Network.getCookies" => json!({
                        "cookies": [{
                            "name": "sessionid",
                            "value": "secret-val",
                            "domain": ".douyin.com",
                            "path": "/",
                            "expires": 1_900_000_000.0,
                            "httpOnly": true,
                            "secure": true,
                            "sameSite": "Lax"
                        }]
                    }),
                    "Page.captureScreenshot" => json!({
                        "data": BASE64.encode(b"\x89PNG\r\n\x1a\n")
                    }),
                    "DOM.getDocument" => json!({ "root": { "nodeId": 1 } }),
                    "DOM.querySelector" => json!({ "nodeId": 9 }),
                    "Fetch.enable" => {
                        let reply = json!({ "id": id, "result": {} });
                        sink.send(Message::text(reply.to_string())).await.unwrap();
                        let paused = json!({
                            "method": "Fetch.requestPaused",
                            "sessionId": session_id,
                            "params": {
                                "requestId": "fetch-1",
                                "resourceType": "Media"
                            }
                        });
                        sink.send(Message::text(paused.to_string())).await.unwrap();
                        continue;
                    }
                    _ => json!({}),
                };

                let reply = json!({ "id": id, "result": result });
                sink.send(Message::text(reply.to_string())).await.unwrap();
            }
        });

        (
            format!("ws://{addr}/devtools/browser/mock"),
            recorded_methods,
        )
    }

    #[tokio::test]
    async fn cdp_client_multiplexes_captures_events_and_never_sends_runtime_enable() {
        let (ws_url, recorded) = spawn_mock_cdp_server().await;
        let conn = Arc::new(
            CdpConnection::connect(&ws_url)
                .await
                .expect("connect mock cdp"),
        );

        // Direct guard: Runtime.enable and Console.enable must be rejected before wire send.
        let rt_err = conn
            .send_command("Runtime.enable", json!({}), None)
            .await
            .expect_err("Runtime.enable must be forbidden");
        assert_eq!(rt_err.code, "CDP_STEALTH_VIOLATION");

        let console_err = conn
            .send_command("Console.enable", json!({}), None)
            .await
            .expect_err("Console.enable must be forbidden");
        assert_eq!(console_err.code, "CDP_STEALTH_VIOLATION");

        let page = CdpPage::attach_or_create(Arc::clone(&conn))
            .await
            .expect("attach page");

        // Concurrent multiplexed requests.
        let (sum_res, iso_res) = tokio::join!(
            page.eval_json("1 + 2", EvalWorld::Main),
            page.eval_json("isolated_probe()", EvalWorld::Isolated),
        );
        assert_eq!(sum_res.expect("main eval"), json!(3));
        assert_eq!(
            iso_res.expect("isolated eval"),
            json!({ "world": "isolated" })
        );

        let url = page
            .goto(
                "https://www.douyin.com/chat",
                LoadWait::DomContentLoaded,
                Duration::from_secs(5),
            )
            .await
            .expect("goto");
        assert_eq!(url, "https://www.douyin.com/chat");

        // In-page fetch, cookies, screenshot, input, file upload.
        let fetch_resp = page
            .fetch_in_page(&FetchRequest::get_text(
                "https://www.douyin.com/aweme/v1/web/test",
            ))
            .await
            .expect("fetch_in_page");
        assert_eq!(fetch_resp.status, 200);
        assert!(fetch_resp.ok);
        assert_eq!(
            fetch_resp.parse_json().expect("json"),
            json!({"status_code": 0})
        );

        let cookies = page
            .cookies(&["https://www.douyin.com/"])
            .await
            .expect("cookies");
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "sessionid");

        let png = page.screenshot_png().await.expect("screenshot");
        assert!(png.starts_with(b"\x89PNG"));

        page.click_xy(100.0, 200.0, MouseButton::Left, 1)
            .await
            .expect("click");
        page.mouse_wheel(100.0, 200.0, 0.0, 120.0)
            .await
            .expect("wheel");
        page.insert_text("hello").await.expect("insert_text");
        page.dispatch_key(&KeyEventSpec::named("Enter"))
            .await
            .expect("dispatch_key");
        page.set_file_input_files("input[type=file]", &[PathBuf::from("photo.png")])
            .await
            .expect("set_file_input_files");

        // Network capture & getResponseBody verification.
        let buffer = page
            .enable_network_capture(vec!["/v1/message/get_message_by_init".to_owned()])
            .await
            .expect("enable network capture");
        let captured = buffer
            .wait_for(Duration::from_secs(3), |ex| ex.response_body.is_some())
            .await
            .expect("captured exchange with response body");
        assert_eq!(captured.status, Some(200));
        assert_eq!(
            captured.request_body.as_deref(),
            Some(b"proto-req".as_slice())
        );
        assert_eq!(
            captured.response_body.as_deref(),
            Some(b"\x08\x01\x12\x04test".as_slice())
        );

        // Media abort verification.
        page.enable_media_abort(MediaBlockMode::MediaOnly)
            .await
            .expect("enable_media_abort");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(page.blocked_media_requests() >= 1);

        page.close().await.expect("close page");

        // Final stealth audit: Runtime.enable and Console.enable were NEVER sent on the wire.
        let methods = recorded.lock().expect("recorded methods").clone();
        assert!(
            !methods
                .iter()
                .any(|m| m == "Runtime.enable" || m == "Console.enable"),
            "stealth violation: forbidden method found in wire log: {methods:?}"
        );
        assert!(methods.contains(&"Page.createIsolatedWorld".to_owned()));
        assert!(methods.contains(&"Network.setBlockedURLs".to_owned()));
        assert!(methods.contains(&"Fetch.failRequest".to_owned()));
    }

    #[tokio::test]
    async fn cdp_connection_surfaces_protocol_errors_and_timeouts() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let ws = tokio_tungstenite::accept_async(stream).await.expect("ws");
            let (mut sink, mut reader) = ws.split();
            while let Some(Ok(Message::Text(text))) = reader.next().await {
                let req: Value = serde_json::from_str(text.as_str()).expect("json");
                let id = req["id"].as_u64().expect("id");
                let method = req["method"].as_str().expect("method");
                if method == "Page.failNow" {
                    let reply = json!({
                        "id": id,
                        "error": { "code": -32000, "message": "synthetic failure" }
                    });
                    sink.send(Message::text(reply.to_string())).await.unwrap();
                }
                // Deliberately ignore "Page.hangForever" to verify timeout path.
            }
        });

        let conn = CdpConnection::connect(&format!("ws://{addr}/devtools/browser/err"))
            .await
            .expect("connect");

        let err = conn
            .send_command("Page.failNow", json!({}), None)
            .await
            .expect_err("must return CDP_COMMAND_ERROR");
        assert_eq!(err.code, "CDP_COMMAND_ERROR");
        assert!(err.message.contains("synthetic failure"));

        let timeout_err = conn
            .send_command_with_timeout(
                "Page.hangForever",
                json!({}),
                None,
                Duration::from_millis(80),
            )
            .await
            .expect_err("must time out");
        assert_eq!(timeout_err.code, "CDP_COMMAND_TIMEOUT");
    }

    #[tokio::test]
    async fn enable_network_capture_merges_patterns_without_duplicate_listeners() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let recorded_methods = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
        let recorded_clone = Arc::clone(&recorded_methods);
        let (event_tx, mut event_rx) = mpsc::channel::<Value>(32);

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let ws = tokio_tungstenite::accept_async(stream).await.expect("ws");
            let (mut sink, mut reader) = ws.split();

            loop {
                tokio::select! {
                    Some(ev) = event_rx.recv() => {
                        sink.send(Message::text(ev.to_string())).await.unwrap();
                    }
                    msg = reader.next() => {
                        let Some(Ok(Message::Text(text))) = msg else {
                            if msg.is_none() {
                                break;
                            }
                            continue;
                        };
                        let req: Value = serde_json::from_str(text.as_str()).expect("json");
                        let id = req["id"].as_u64().expect("id");
                        let method = req["method"].as_str().expect("method").to_owned();
                        let req_id_param = req
                            .pointer("/params/requestId")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        recorded_clone
                            .lock()
                            .expect("recorded lock")
                            .push((method.clone(), req_id_param.clone()));

                        let result = match method.as_str() {
                            "Target.getTargets" => json!({
                                "targetInfos": [{ "targetId": "t-1", "type": "page" }]
                            }),
                            "Target.attachToTarget" => json!({ "sessionId": "s-1" }),
                            "Page.getFrameTree" => json!({
                                "frameTree": { "frame": { "id": "f-1" } }
                            }),
                            "Network.getResponseBody" => match req_id_param.as_deref() {
                                Some("req-search") => json!({
                                    "body": r#"{"status_code":0}"#,
                                    "base64Encoded": false
                                }),
                                Some("req-empty") => json!({
                                    "body": "",
                                    "base64Encoded": false
                                }),
                                _ => json!({
                                    "body": "extra",
                                    "base64Encoded": false
                                }),
                            },
                            _ => json!({}),
                        };
                        let reply = json!({ "id": id, "result": result });
                        sink.send(Message::text(reply.to_string())).await.unwrap();
                    }
                }
            }
        });

        let conn = Arc::new(
            CdpConnection::connect(&format!("ws://{addr}/devtools/browser/dedup"))
                .await
                .expect("connect"),
        );
        let page = CdpPage::attach_or_create(Arc::clone(&conn))
            .await
            .expect("attach");

        // Call enable_network_capture twice with distinct patterns.
        let buf1 = page
            .enable_network_capture(vec!["/aweme/v1/web/discover/search".to_owned()])
            .await
            .expect("first enable");
        let buf2 = page
            .enable_network_capture(vec!["/aweme/v1/web/aweme/post".to_owned()])
            .await
            .expect("second enable");

        let emit_exchange = |tx: &mpsc::Sender<Value>, req_id: &str, url: &str| {
            let tx = tx.clone();
            let req_id = req_id.to_owned();
            let url = url.to_owned();
            async move {
                tx.send(json!({
                    "method": "Network.requestWillBeSent",
                    "sessionId": "s-1",
                    "params": {
                        "requestId": req_id,
                        "type": "XHR",
                        "request": { "url": url, "method": "GET", "headers": {} }
                    }
                }))
                .await
                .unwrap();
                tx.send(json!({
                    "method": "Network.responseReceived",
                    "sessionId": "s-1",
                    "params": {
                        "requestId": req_id,
                        "type": "XHR",
                        "response": {
                            "url": url,
                            "status": 200,
                            "mimeType": "application/json",
                            "headers": { "Content-Type": "application/json" }
                        }
                    }
                }))
                .await
                .unwrap();
                tx.send(json!({
                    "method": "Network.loadingFinished",
                    "sessionId": "s-1",
                    "params": { "requestId": req_id }
                }))
                .await
                .unwrap();
            }
        };

        emit_exchange(
            &event_tx,
            "req-search",
            "https://www.douyin.com/aweme/v1/web/discover/search/?keyword=test",
        )
        .await;
        emit_exchange(
            &event_tx,
            "req-ignored",
            "https://www.douyin.com/static/app.js",
        )
        .await;
        emit_exchange(
            &event_tx,
            "req-empty",
            "https://www.douyin.com/aweme/v1/web/aweme/post/?sec_user_id=u1",
        )
        .await;

        let search_ex = buf1
            .wait_for(Duration::from_secs(2), |ex| {
                ex.request_id == "req-search" && ex.response_body.is_some()
            })
            .await
            .expect("req-search captured");
        assert_eq!(
            search_ex.response_body.as_deref(),
            Some(br#"{"status_code":0}"#.as_slice())
        );

        let empty_ex = buf2
            .wait_for(Duration::from_secs(2), |ex| {
                ex.request_id == "req-empty" && ex.response_body.is_some()
            })
            .await
            .expect("req-empty captured with Some(empty)");
        assert_eq!(empty_ex.response_body.as_deref(), Some(b"".as_slice()));

        // Allow any duplicate tasks (if they existed) time to run before auditing.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let snap = buf2.snapshot();
        assert_eq!(
            snap.len(),
            2,
            "expected exactly 2 captured exchanges: {snap:?}"
        );
        assert!(snap.iter().any(|ex| ex.request_id == "req-search"));
        assert!(snap.iter().any(|ex| ex.request_id == "req-empty"));

        let calls = recorded_methods.lock().expect("recorded lock").clone();
        let network_enables = calls.iter().filter(|(m, _)| m == "Network.enable").count();
        assert_eq!(
            network_enables, 1,
            "Network.enable should only be called once, got {network_enables}"
        );
        let search_body_fetches = calls
            .iter()
            .filter(|(m, id)| m == "Network.getResponseBody" && id.as_deref() == Some("req-search"))
            .count();
        let empty_body_fetches = calls
            .iter()
            .filter(|(m, id)| m == "Network.getResponseBody" && id.as_deref() == Some("req-empty"))
            .count();
        assert_eq!(
            search_body_fetches, 1,
            "duplicate listener fetched req-search body {search_body_fetches} times"
        );
        assert_eq!(
            empty_body_fetches, 1,
            "duplicate listener fetched req-empty body {empty_body_fetches} times"
        );

        // Passing empty url_patterns upgrades to capture-all without spawning another task.
        let _ = page
            .enable_network_capture(Vec::new())
            .await
            .expect("capture all");
        let _ = page
            .enable_network_capture(vec!["/only-something-else".to_owned()])
            .await
            .expect("still capture all");
        emit_exchange(
            &event_tx,
            "req-any",
            "https://www.douyin.com/unmatched/endpoint",
        )
        .await;
        let any_ex = buf2
            .wait_for(Duration::from_secs(2), |ex| {
                ex.request_id == "req-any" && ex.response_body.is_some()
            })
            .await
            .expect("capture-all should capture unmatched endpoint");
        assert_eq!(any_ex.response_body.as_deref(), Some(b"extra".as_slice()));

        page.close().await.expect("close");
    }
}
