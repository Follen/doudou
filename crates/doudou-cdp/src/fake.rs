//! Deterministic in-memory fakes ([`FakeCdpPage`] and [`FakeBrowserSession`]) for testing
//! without launching Chrome.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use doudou_core::{DoudouError, ProfilePaths, Result};
use serde_json::{Value, json};

use crate::cdp::{
    CapturedNetworkExchange, CdpPageOps, Cookie, EvalWorld, FetchRequest, FetchResponse,
    KeyEventSpec, LoadWait, MediaBlockMode, MouseButton, NetworkCaptureBuffer,
};
use crate::session::{
    AuthPolicy, ChromeSession, LoginClassification, NavigationResult, SessionMode,
};

/// Recorded JavaScript evaluation call made against [`FakeCdpPage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedEval {
    /// Evaluated JS expression.
    pub expression: String,
    /// Target execution world (`Main` or `Isolated`).
    pub world: EvalWorld,
}

/// Recorded mouse click call made against [`FakeCdpPage`].
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedClick {
    /// Viewport X coordinate.
    pub x: f64,
    /// Viewport Y coordinate.
    pub y: f64,
    /// Mouse button.
    pub button: MouseButton,
    /// Click count.
    pub click_count: u32,
}

/// Recorded mouse wheel call made against [`FakeCdpPage`].
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedWheel {
    /// Viewport X coordinate.
    pub x: f64,
    /// Viewport Y coordinate.
    pub y: f64,
    /// Horizontal scroll delta.
    pub delta_x: f64,
    /// Vertical scroll delta.
    pub delta_y: f64,
}

/// Recorded file upload made against [`FakeCdpPage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedFileUpload {
    /// Target CSS selector.
    pub css_selector: String,
    /// Uploaded local file paths.
    pub files: Vec<PathBuf>,
}

#[derive(Debug)]
struct FakePageState {
    url: String,
    title: String,
    cookies: Vec<Cookie>,
    screenshot_png: Vec<u8>,
    blocked_media_requests: usize,
    eval_rules: Vec<(String, VecDeque<Value>)>,
    eval_fallback: Value,
    fetch_rules: Vec<(String, VecDeque<Result<FetchResponse>>)>,
    navigations: Vec<String>,
    evaluations: Vec<RecordedEval>,
    fetches: Vec<FetchRequest>,
    clicks: Vec<RecordedClick>,
    mouse_moves: Vec<(f64, f64)>,
    mouse_wheels: Vec<RecordedWheel>,
    inserted_texts: Vec<String>,
    dispatched_keys: Vec<KeyEventSpec>,
    uploaded_files: Vec<RecordedFileUpload>,
    media_abort_modes: Vec<MediaBlockMode>,
    network_capture_patterns: Vec<Vec<String>>,
    closed: bool,
}

impl Default for FakePageState {
    fn default() -> Self {
        Self {
            url: "about:blank".to_owned(),
            title: "Fake Chrome".to_owned(),
            cookies: Vec::new(),
            screenshot_png: b"\x89PNG\r\n\x1a\nFAKE".to_vec(),
            blocked_media_requests: 0,
            eval_rules: Vec::new(),
            eval_fallback: Value::Null,
            fetch_rules: Vec::new(),
            navigations: Vec::new(),
            evaluations: Vec::new(),
            fetches: Vec::new(),
            clicks: Vec::new(),
            mouse_moves: Vec::new(),
            mouse_wheels: Vec::new(),
            inserted_texts: Vec::new(),
            dispatched_keys: Vec::new(),
            uploaded_files: Vec::new(),
            media_abort_modes: Vec::new(),
            network_capture_patterns: Vec::new(),
            closed: false,
        }
    }
}

/// Deterministic fake implementing [`CdpPageOps`] with scripted responses and call recording.
#[derive(Debug, Clone)]
pub struct FakeCdpPage {
    state: Arc<Mutex<FakePageState>>,
    capture_buffer: NetworkCaptureBuffer,
}

impl Default for FakeCdpPage {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeCdpPage {
    /// Create a fresh [`FakeCdpPage`] at `about:blank`.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakePageState::default())),
            capture_buffer: NetworkCaptureBuffer::new(),
        }
    }

    /// Set the current URL directly.
    pub fn set_url(&self, url: impl Into<String>) {
        self.state.lock().expect("fake page lock poisoned").url = url.into();
    }

    /// Set the current document title.
    pub fn set_title(&self, title: impl Into<String>) {
        self.state.lock().expect("fake page lock poisoned").title = title.into();
    }

    /// Replace the stored cookie jar.
    pub fn set_cookies(&self, cookies: Vec<Cookie>) {
        self.state.lock().expect("fake page lock poisoned").cookies = cookies;
    }

    /// Set the PNG bytes returned by `screenshot_png`.
    pub fn set_screenshot_png(&self, png: Vec<u8>) {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .screenshot_png = png;
    }

    /// Set the fallback JSON value returned by `eval_json` when no substring rule matches.
    pub fn set_eval_fallback(&self, value: Value) {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .eval_fallback = value;
    }

    /// Enqueue a scripted `eval_json` return value for any expression containing `expr_substring`.
    ///
    /// When the queue has a single remaining value, it repeats on subsequent calls; push multiple
    /// values to script state transitions (e.g. verification modal present -> cleared).
    pub fn push_eval_result(&self, expr_substring: impl Into<String>, value: Value) {
        let key = expr_substring.into();
        let mut guard = self.state.lock().expect("fake page lock poisoned");
        if let Some((_, queue)) = guard.eval_rules.iter_mut().find(|(k, _)| k == &key) {
            queue.push_back(value);
        } else {
            let mut queue = VecDeque::new();
            queue.push_back(value);
            guard.eval_rules.push((key, queue));
        }
    }

    /// Enqueue a scripted `fetch_in_page` outcome for any request URL containing `url_substring`.
    pub fn push_fetch_result(
        &self,
        url_substring: impl Into<String>,
        outcome: Result<FetchResponse>,
    ) {
        let key = url_substring.into();
        let mut guard = self.state.lock().expect("fake page lock poisoned");
        if let Some((_, queue)) = guard.fetch_rules.iter_mut().find(|(k, _)| k == &key) {
            queue.push_back(outcome);
        } else {
            let mut queue = VecDeque::new();
            queue.push_back(outcome);
            guard.fetch_rules.push((key, queue));
        }
    }

    /// Insert a synthetic captured network exchange into the page's capture buffer.
    pub fn push_captured_exchange(&self, exchange: CapturedNetworkExchange) {
        self.capture_buffer.upsert(exchange);
    }

    /// Set the simulated blocked media request count.
    pub fn set_blocked_media_requests(&self, count: usize) {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .blocked_media_requests = count;
    }

    /// Return all URLs passed to `goto`.
    pub fn navigations(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .navigations
            .clone()
    }

    /// Return all recorded JavaScript evaluations.
    pub fn evaluations(&self) -> Vec<RecordedEval> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .evaluations
            .clone()
    }

    /// Return all recorded in-page fetch requests.
    pub fn fetches(&self) -> Vec<FetchRequest> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .fetches
            .clone()
    }

    /// Return all recorded clicks.
    pub fn clicks(&self) -> Vec<RecordedClick> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .clicks
            .clone()
    }

    /// Return all recorded mouse moves.
    pub fn mouse_moves(&self) -> Vec<(f64, f64)> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .mouse_moves
            .clone()
    }

    /// Return all recorded mouse wheel scrolls.
    pub fn mouse_wheels(&self) -> Vec<RecordedWheel> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .mouse_wheels
            .clone()
    }

    /// Return all recorded `insert_text` payloads.
    pub fn inserted_texts(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .inserted_texts
            .clone()
    }

    /// Return all recorded keyboard events.
    pub fn dispatched_keys(&self) -> Vec<KeyEventSpec> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .dispatched_keys
            .clone()
    }

    /// Return all recorded file uploads.
    pub fn uploaded_files(&self) -> Vec<RecordedFileUpload> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .uploaded_files
            .clone()
    }

    /// Return all configured media abort modes.
    pub fn media_abort_modes(&self) -> Vec<MediaBlockMode> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .media_abort_modes
            .clone()
    }

    /// Return all configured network capture pattern lists.
    pub fn network_capture_patterns(&self) -> Vec<Vec<String>> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .network_capture_patterns
            .clone()
    }

    /// Return whether `close()` has been called.
    pub fn is_closed(&self) -> bool {
        self.state.lock().expect("fake page lock poisoned").closed
    }
}

#[async_trait]
impl CdpPageOps for FakeCdpPage {
    async fn goto(&self, url: &str, _wait: LoadWait, _timeout: Duration) -> Result<String> {
        let mut guard = self.state.lock().expect("fake page lock poisoned");
        url.clone_into(&mut guard.url);
        guard.navigations.push(url.to_owned());
        Ok(guard.url.clone())
    }

    async fn wait_for_load(&self, _wait: LoadWait, _timeout: Duration) -> Result<()> {
        Ok(())
    }

    async fn current_url(&self) -> Result<String> {
        Ok(self
            .state
            .lock()
            .expect("fake page lock poisoned")
            .url
            .clone())
    }

    async fn title(&self) -> Result<String> {
        Ok(self
            .state
            .lock()
            .expect("fake page lock poisoned")
            .title
            .clone())
    }

    async fn eval_json(&self, expression: &str, world: EvalWorld) -> Result<Value> {
        let mut guard = self.state.lock().expect("fake page lock poisoned");
        guard.evaluations.push(RecordedEval {
            expression: expression.to_owned(),
            world,
        });
        for (pattern, queue) in &mut guard.eval_rules {
            if expression.contains(pattern.as_str()) {
                if queue.len() > 1 {
                    if let Some(val) = queue.pop_front() {
                        return Ok(val);
                    }
                } else if let Some(val) = queue.front() {
                    return Ok(val.clone());
                }
            }
        }
        Ok(guard.eval_fallback.clone())
    }

    async fn fetch_in_page(&self, request: &FetchRequest) -> Result<FetchResponse> {
        let mut guard = self.state.lock().expect("fake page lock poisoned");
        guard.fetches.push(request.clone());
        for (pattern, queue) in &mut guard.fetch_rules {
            if request.url.contains(pattern.as_str()) {
                if queue.len() > 1 {
                    if let Some(outcome) = queue.pop_front() {
                        return outcome;
                    }
                } else if let Some(outcome) = queue.front() {
                    return outcome.clone();
                }
            }
        }
        Err(DoudouError::new(
            "FAKE_FETCH_UNMATCHED",
            format!("No fake fetch response registered for URL: {}", request.url),
        ))
    }

    async fn cookies(&self, urls: &[&str]) -> Result<Vec<Cookie>> {
        let guard = self.state.lock().expect("fake page lock poisoned");
        if urls.is_empty() {
            return Ok(guard.cookies.clone());
        }
        let filtered = guard
            .cookies
            .iter()
            .filter(|cookie| {
                let domain = cookie.domain.trim_start_matches('.');
                urls.iter().any(|u| u.contains(domain))
            })
            .cloned()
            .collect();
        Ok(filtered)
    }

    async fn click_xy(&self, x: f64, y: f64, button: MouseButton, click_count: u32) -> Result<()> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .clicks
            .push(RecordedClick {
                x,
                y,
                button,
                click_count,
            });
        Ok(())
    }

    async fn mouse_move(&self, x: f64, y: f64) -> Result<()> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .mouse_moves
            .push((x, y));
        Ok(())
    }

    async fn mouse_wheel(&self, x: f64, y: f64, delta_x: f64, delta_y: f64) -> Result<()> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .mouse_wheels
            .push(RecordedWheel {
                x,
                y,
                delta_x,
                delta_y,
            });
        Ok(())
    }

    async fn insert_text(&self, text: &str) -> Result<()> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .inserted_texts
            .push(text.to_owned());
        Ok(())
    }

    async fn dispatch_key(&self, spec: &KeyEventSpec) -> Result<()> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .dispatched_keys
            .push(spec.clone());
        Ok(())
    }

    async fn set_file_input_files(&self, css_selector: &str, files: &[PathBuf]) -> Result<()> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .uploaded_files
            .push(RecordedFileUpload {
                css_selector: css_selector.to_owned(),
                files: files.to_vec(),
            });
        Ok(())
    }

    async fn screenshot_png(&self) -> Result<Vec<u8>> {
        Ok(self
            .state
            .lock()
            .expect("fake page lock poisoned")
            .screenshot_png
            .clone())
    }

    async fn enable_network_capture(
        &self,
        url_patterns: Vec<String>,
    ) -> Result<NetworkCaptureBuffer> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .network_capture_patterns
            .push(url_patterns);
        Ok(self.capture_buffer.clone())
    }

    async fn enable_media_abort(&self, mode: MediaBlockMode) -> Result<()> {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .media_abort_modes
            .push(mode);
        Ok(())
    }

    fn blocked_media_requests(&self) -> usize {
        self.state
            .lock()
            .expect("fake page lock poisoned")
            .blocked_media_requests
    }

    fn capture_buffer(&self) -> NetworkCaptureBuffer {
        self.capture_buffer.clone()
    }

    async fn close(&self) -> Result<()> {
        self.state.lock().expect("fake page lock poisoned").closed = true;
        Ok(())
    }
}

/// Deterministic fake session wrapping a [`FakeCdpPage`] and [`ChromeSession`] for tests.
#[derive(Debug)]
pub struct FakeBrowserSession {
    page: Arc<FakeCdpPage>,
    session: ChromeSession,
}

impl FakeBrowserSession {
    /// Construct a fake browser session for `paths` with an authenticated default state.
    pub fn authenticated(paths: ProfilePaths) -> Self {
        let page = Arc::new(FakeCdpPage::new());
        page.set_url("https://www.douyin.com/chat");
        page.set_cookies(vec![Cookie {
            name: "sessionid".to_owned(),
            value: "fake-session-token".to_owned(),
            domain: ".douyin.com".to_owned(),
            path: "/".to_owned(),
            expires: -1.0,
            http_only: true,
            secure: true,
            same_site: Some("Lax".to_owned()),
        }]);
        page.push_eval_result("conversation-item", json!(true));
        page.push_eval_result("second-verify-mask", Value::Null);
        page.push_captured_exchange(CapturedNetworkExchange {
            request_id: "fake-init-1".to_owned(),
            url: "https://imapi.douyin.com/v1/message/get_message_by_init".to_owned(),
            method: "POST".to_owned(),
            status: Some(200),
            mime_type: Some("application/x-protobuf".to_owned()),
            request_headers: BTreeMap::new(),
            response_headers: BTreeMap::new(),
            request_body: Some(b"\x08\x01".to_vec()),
            response_body: Some(b"\x08\x02\x12\x02ok".to_vec()),
            resource_type: Some("Fetch".to_owned()),
            timestamp: doudou_core::utc_now(),
        });
        Self::from_page(paths, page)
    }

    /// Construct a fake browser session backed by a custom [`FakeCdpPage`].
    pub fn from_page(paths: ProfilePaths, page: Arc<FakeCdpPage>) -> Self {
        let page_dyn: Arc<dyn CdpPageOps> = Arc::clone(&page) as Arc<dyn CdpPageOps>;
        let session = ChromeSession::with_page(paths, page_dyn, SessionMode::Fake);
        Self { page, session }
    }

    /// Access the underlying [`FakeCdpPage`] to script responses or inspect recorded calls.
    pub fn fake_page(&self) -> &Arc<FakeCdpPage> {
        &self.page
    }

    /// Access the underlying [`ChromeSession`].
    pub fn session(&self) -> &ChromeSession {
        &self.session
    }

    /// Mutably access the underlying [`ChromeSession`].
    pub fn session_mut(&mut self) -> &mut ChromeSession {
        &mut self.session
    }

    /// Consume this wrapper and return the inner [`ChromeSession`].
    pub fn into_chrome_session(self) -> ChromeSession {
        self.session
    }

    /// Convenience delegate to [`ChromeSession::navigate_douyin`].
    pub async fn navigate_douyin(&mut self, route_or_url: &str) -> Result<NavigationResult> {
        self.session.navigate_douyin(route_or_url).await
    }

    /// Convenience delegate to [`ChromeSession::ensure_authenticated`].
    pub async fn ensure_authenticated(
        &mut self,
        policy: AuthPolicy,
    ) -> Result<LoginClassification> {
        self.session.ensure_authenticated(policy).await
    }

    /// Convenience delegate to [`ChromeSession::page_fetch`].
    pub async fn page_fetch(&mut self, request: &FetchRequest) -> Result<FetchResponse> {
        self.session.page_fetch(request).await
    }
}

#[cfg(test)]
mod tests {
    use doudou_core::profile_paths;
    use tempfile::TempDir;

    use super::*;
    use crate::session::{LoginStatus, VerificationKind};

    fn test_paths() -> (TempDir, ProfilePaths) {
        let dir = TempDir::new().expect("tempdir");
        let paths = profile_paths(&dir.path().join("home"), &dir.path().join("proj"), "work")
            .expect("profile paths");
        (dir, paths)
    }

    #[tokio::test]
    async fn fake_cdp_page_records_all_interactions_and_serves_scripted_outputs() {
        let page = FakeCdpPage::new();
        page.push_eval_result("1 + 1", json!(2));
        page.push_fetch_result(
            "/aweme/v1/web/im",
            Ok(FetchResponse {
                status: 200,
                ok: true,
                headers: BTreeMap::new(),
                body_text: Some("{\"ok\":true}".to_owned()),
                body_base64: None,
            }),
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
        assert_eq!(page.navigations(), vec!["https://www.douyin.com/chat"]);

        let val = page
            .eval_json("1 + 1", EvalWorld::Isolated)
            .await
            .expect("eval");
        assert_eq!(val, json!(2));
        assert_eq!(page.evaluations()[0].world, EvalWorld::Isolated);

        let resp = page
            .fetch_in_page(&FetchRequest::get_text(
                "https://www.douyin.com/aweme/v1/web/im",
            ))
            .await
            .expect("fetch");
        assert_eq!(resp.status, 200);
        assert_eq!(page.fetches().len(), 1);

        page.click_xy(12.5, 34.5, MouseButton::Right, 1)
            .await
            .expect("click");
        page.mouse_move(10.0, 20.0).await.expect("move");
        page.mouse_wheel(10.0, 20.0, 0.0, 300.0)
            .await
            .expect("wheel");
        page.insert_text("你好").await.expect("insert_text");
        page.dispatch_key(&KeyEventSpec::named("Enter"))
            .await
            .expect("key");
        page.set_file_input_files("input[type=file]", &[PathBuf::from("a.png")])
            .await
            .expect("files");
        page.enable_media_abort(MediaBlockMode::HeavyAssets)
            .await
            .expect("media abort");
        let _ = page
            .enable_network_capture(vec!["/v1/message".to_owned()])
            .await
            .expect("capture");

        assert_eq!(page.clicks().len(), 1);
        assert_eq!(page.mouse_moves(), vec![(10.0, 20.0)]);
        assert_eq!(page.mouse_wheels().len(), 1);
        assert_eq!(page.inserted_texts(), vec!["你好"]);
        assert_eq!(page.dispatched_keys()[0].key, "Enter");
        assert_eq!(page.uploaded_files()[0].css_selector, "input[type=file]");
        assert_eq!(page.media_abort_modes(), vec![MediaBlockMode::HeavyAssets]);
        assert_eq!(page.network_capture_patterns(), vec![vec!["/v1/message"]]);

        page.close().await.expect("close");
        assert!(page.is_closed());
    }

    #[tokio::test]
    async fn fake_browser_session_authenticates_navigates_and_fetches() {
        let (_dir, paths) = test_paths();
        let mut fake = FakeBrowserSession::authenticated(paths);
        fake.fake_page().push_fetch_result(
            "https://www.douyin.com/aweme/v1/web/user/profile",
            Ok(FetchResponse {
                status: 200,
                ok: true,
                headers: BTreeMap::new(),
                body_text: Some("{\"user\":{\"nickname\":\"Alice\"}}".to_owned()),
                body_base64: None,
            }),
        );

        let auth = fake
            .ensure_authenticated(AuthPolicy::FailFast)
            .await
            .expect("authenticated");
        assert!(auth.valid);
        assert_eq!(auth.status, LoginStatus::Valid);
        assert!(auth.im_initialized);

        let nav = fake.navigate_douyin("/user/self").await.expect("nav");
        assert_eq!(nav.url, "https://www.douyin.com/user/self");

        let resp = fake
            .page_fetch(&FetchRequest::get_text(
                "https://www.douyin.com/aweme/v1/web/user/profile",
            ))
            .await
            .expect("page_fetch");
        assert_eq!(
            resp.parse_json().expect("json")["user"]["nickname"],
            "Alice"
        );

        // Rejects non-Douyin origins for page_fetch and navigate_douyin.
        let err = fake
            .page_fetch(&FetchRequest::get_text("https://example.com/"))
            .await
            .expect_err("non-douyin fetch must fail");
        assert_eq!(err.code, "PROTOCOL_ENDPOINT_INVALID");

        let nav_err = fake
            .navigate_douyin("https://example.com/phish")
            .await
            .expect_err("non-douyin navigation must fail");
        assert_eq!(nav_err.code, "INVALID_DOUYIN_ORIGIN");
    }

    #[tokio::test]
    async fn session_switches_to_headed_takeover_and_completes_when_modal_clears() {
        let (_dir, paths) = test_paths();
        let page = Arc::new(FakeCdpPage::new());
        page.set_url("https://www.douyin.com/chat");
        // First check sees slider captcha; second check sees captcha cleared and chat loaded.
        page.push_eval_result("second-verify-mask", json!("slider"));
        page.push_eval_result("second-verify-mask", Value::Null);
        page.push_eval_result("conversation-item", json!(false));
        page.push_eval_result("conversation-item", json!(true));

        let mut fake = FakeBrowserSession::from_page(paths, Arc::clone(&page));

        // FailFast surfaces VERIFICATION_REQUIRED with modal kind.
        let fail_fast_err = fake
            .ensure_authenticated(AuthPolicy::FailFast)
            .await
            .expect_err("fail fast should report verification required");
        assert_eq!(fail_fast_err.code, "VERIFICATION_REQUIRED");
        assert_eq!(
            fail_fast_err
                .details
                .as_ref()
                .and_then(|d| d.get("verification_kind"))
                .and_then(Value::as_str),
            Some("slider")
        );

        // Re-script slider -> cleared transition for InteractiveTakeover.
        let page2 = Arc::new(FakeCdpPage::new());
        page2.set_url("https://www.douyin.com/chat");
        page2.push_eval_result("second-verify-mask", json!("qr_login"));
        page2.push_eval_result("second-verify-mask", Value::Null);
        page2.push_eval_result("conversation-item", json!(false));
        page2.push_eval_result("conversation-item", json!(true));

        let (_dir2, paths2) = test_paths();
        let mut takeover_fake = FakeBrowserSession::from_page(paths2, page2);
        let result = takeover_fake
            .ensure_authenticated(AuthPolicy::InteractiveTakeover {
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            })
            .await
            .expect("takeover should succeed after modal clears");
        assert!(result.valid);
        assert_eq!(result.verification_kind, None);
        assert!(!takeover_fake.session().takeover().active);
        assert!(takeover_fake.session().status().await.mode_transition_count >= 2);
        assert_eq!(
            VerificationKind::from_tag("quick_login_sms"),
            Some(VerificationKind::QuickLoginSms)
        );
    }

    #[tokio::test]
    async fn post_protobuf_retries_transient_network_errors() {
        let (_dir, paths) = test_paths();
        let mut fake = FakeBrowserSession::authenticated(paths);
        fake.fake_page().push_fetch_result(
            "https://imapi.douyin.com/v1/message/send",
            Err(DoudouError::new(
                "CDP_EVAL_ERROR",
                "TypeError: NetworkError when attempting to fetch",
            )),
        );
        fake.fake_page().push_fetch_result(
            "https://imapi.douyin.com/v1/message/send",
            Ok(FetchResponse {
                status: 200,
                ok: true,
                headers: BTreeMap::new(),
                body_text: None,
                body_base64: Some("CAESAm9r".to_owned()),
            }),
        );

        let (status, bytes) = fake
            .session_mut()
            .post_protobuf("https://imapi.douyin.com/v1/message/send", b"\x08\x01")
            .await
            .expect("retry should succeed on second attempt");
        assert_eq!(status, 200);
        assert_eq!(bytes, b"\x08\x01\x12\x02ok");
        assert_eq!(fake.fake_page().fetches().len(), 2);
    }

    #[tokio::test]
    async fn takeover_times_out_and_im_init_rejects_json_payloads() {
        let (_dir, paths) = test_paths();
        let page = Arc::new(FakeCdpPage::new());
        page.set_url("https://www.douyin.com/chat");
        page.push_eval_result("second-verify-mask", json!("qr_login"));
        page.push_eval_result("conversation-item", json!(false));
        // JSON body on IM init must NOT count as valid binary Protobuf IM init.
        page.push_captured_exchange(CapturedNetworkExchange {
            request_id: "json-init".to_owned(),
            url: "https://imapi.douyin.com/v1/message/get_message_by_init".to_owned(),
            method: "POST".to_owned(),
            status: Some(200),
            mime_type: Some("application/json".to_owned()),
            request_headers: BTreeMap::new(),
            response_headers: BTreeMap::new(),
            request_body: Some(b"req".to_vec()),
            response_body: Some(b"{\"status_code\":8}".to_vec()),
            resource_type: Some("Fetch".to_owned()),
            timestamp: doudou_core::utc_now(),
        });

        let mut fake = FakeBrowserSession::from_page(paths, page);
        assert!(fake.session().captured_im_init().is_none());

        let err = fake
            .ensure_authenticated(AuthPolicy::InteractiveTakeover {
                timeout: Duration::from_millis(60),
                poll_interval: Duration::from_millis(15),
            })
            .await
            .expect_err("must time out");
        assert_eq!(err.code, "LOGIN_TIMEOUT");
    }
}
