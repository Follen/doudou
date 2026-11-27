//! High-level Douyin browser session managing headed/headless Chrome lifecycle,
//! authentication detection, interactive takeover, in-page fetch, and network capture.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use doudou_core::{DoudouError, ProfilePaths, Result, atomic_write_json, read_json, utc_now};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, info};

use crate::cdp::{
    CapturedNetworkExchange, CdpBrowser, CdpPageOps, EvalWorld, FetchRequest, FetchResponse,
    LoadWait, MediaBlockMode, NetworkCaptureBuffer,
};
use crate::chrome::{ChromeLaunchOptions, DEFAULT_STARTUP_TIMEOUT, LaunchMode};

/// Douyin web home origin.
pub const DOUYIN_WEB_ORIGIN: &str = "https://www.douyin.com";

/// Douyin IM API origin for Protobuf transport.
pub const DOUYIN_IM_ORIGIN: &str = "https://imapi.douyin.com";

/// IM initialization endpoint path captured during chat bootstrap.
pub const IM_INIT_PATH_MARKER: &str = "/v1/message/get_message_by_init";

/// IM user info endpoint path captured during chat bootstrap.
pub const IM_USER_INFO_PATH_MARKER: &str = "/aweme/v1/web/im/user/info/";

/// Bounded retry delays for transient in-page network failures.
const PROTOBUF_RETRY_DELAYS: &[Duration] =
    &[Duration::from_millis(250), Duration::from_millis(500)];

/// High-level session execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// Background stealth headless Chrome (`--headless=new`).
    Headless,
    /// Visible headed Chrome window for interactive login or captcha completion.
    HeadedTakeover,
    /// In-memory deterministic fake page for tests.
    Fake,
}

impl SessionMode {
    fn to_launch_mode(self) -> LaunchMode {
        match self {
            Self::HeadedTakeover => LaunchMode::Headed,
            Self::Headless | Self::Fake => LaunchMode::Headless,
        }
    }
}

/// Interactive login or verification challenge detected on a Douyin page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum VerificationKind {
    /// SMS verification code modal.
    SmsCode,
    /// Quick-login SMS modal.
    QuickLoginSms,
    /// Slider / drag captcha iframe or modal.
    Slider,
    /// QR code login modal.
    QrLogin,
    /// Passkey / security key prompt.
    Passkey,
    /// Generic verification dialog or mask.
    GenericVerification,
}

impl VerificationKind {
    /// Parse a verification kind tag returned by DOM inspection.
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "sms_code" => Some(Self::SmsCode),
            "quick_login_sms" => Some(Self::QuickLoginSms),
            "slider" => Some(Self::Slider),
            "qr_login" => Some(Self::QrLogin),
            "passkey" => Some(Self::Passkey),
            "generic_verification" => Some(Self::GenericVerification),
            _ => None,
        }
    }
}

/// Summary login state for a Douyin profile session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginStatus {
    /// Authenticated session verified.
    Valid,
    /// User must log in (e.g. QR login modal or unauthenticated state).
    LoginRequired,
    /// Interactive captcha/SMS/security verification is blocking the page.
    VerificationRequired,
}

/// Detailed authentication classification result for the active page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginClassification {
    /// Coarse login state.
    pub status: LoginStatus,
    /// True only when `status == LoginStatus::Valid`.
    pub valid: bool,
    /// Machine-readable reason code (`LOGIN_REQUIRED` / `VERIFICATION_REQUIRED`), if not valid.
    pub reason_code: Option<String>,
    /// Total non-empty cookies on Douyin origins.
    pub cookie_count: usize,
    /// Authenticated cookie count (`cookie_count` when valid, `0` otherwise).
    pub authenticated_cookie_count: usize,
    /// Whether binary IM init request + response have been captured.
    pub im_initialized: bool,
    /// Whether the `/chat` conversation list is loaded in the DOM.
    pub chat_loaded: bool,
    /// Whether any login or verification modal is currently visible.
    pub verification_visible: bool,
    /// Specific modal kind when `verification_visible` is true.
    pub verification_kind: Option<VerificationKind>,
}

/// Interactive takeover status tracked by [`ChromeSession`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeoverState {
    /// Whether headed interactive takeover is currently active.
    pub active: bool,
    /// Machine-readable reason code triggering takeover.
    pub reason: Option<String>,
    /// Detected verification challenge kind.
    pub verification_kind: Option<VerificationKind>,
    /// UTC timestamp when takeover started.
    pub started_at: Option<String>,
}

/// Record of a headless <-> headed browser context transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeTransition {
    /// UTC timestamp of transition.
    pub at: String,
    /// Previous session mode.
    pub from: SessionMode,
    /// Target session mode.
    pub to: SessionMode,
    /// Active page URL after transition, if any.
    pub page_url: Option<String>,
}

/// Result returned by [`ChromeSession::navigate_douyin`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationResult {
    /// Final resolved URL after navigation.
    pub url: String,
    /// Document title after navigation.
    pub title: String,
}

/// Snapshot of runtime telemetry for [`ChromeSession`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStatus {
    /// Whether a browser/page handle is currently active.
    pub running: bool,
    /// Active session mode.
    pub mode: SessionMode,
    /// Timestamp when the session started.
    pub started_at: Option<String>,
    /// Current page URL if available.
    pub page_url: Option<String>,
    /// Count of requests aborted by media blocking.
    pub blocked_media_requests: usize,
    /// Count of browser context generations launched.
    pub context_generation: u64,
    /// Count of headless/headed mode transitions.
    pub mode_transition_count: u64,
    /// Current interactive takeover state.
    pub takeover: TakeoverState,
    /// Count of buffered network exchanges.
    pub captured_exchange_count: usize,
}

/// Configuration for creating a [`ChromeSession`].
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Resolved profile paths (uses `paths.browser_data` as `--user-data-dir`).
    pub paths: ProfilePaths,
    /// Initial launch mode (`Headless` by default).
    pub initial_mode: SessionMode,
    /// Optional explicit Chrome binary override.
    pub chrome_binary: Option<PathBuf>,
    /// Media/asset blocking policy applied on page startup.
    pub media_block: Option<MediaBlockMode>,
    /// Substring patterns for automatic network response capture.
    pub capture_url_patterns: Vec<String>,
    /// Browser startup timeout.
    pub startup_timeout: Duration,
}

impl SessionConfig {
    /// Construct a default headless session configuration for `paths`.
    pub fn new(paths: ProfilePaths) -> Self {
        Self {
            paths,
            initial_mode: SessionMode::Headless,
            chrome_binary: None,
            media_block: Some(MediaBlockMode::MediaOnly),
            capture_url_patterns: vec![
                IM_INIT_PATH_MARKER.to_owned(),
                IM_USER_INFO_PATH_MARKER.to_owned(),
            ],
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
        }
    }

    /// Set the initial session mode.
    #[must_use]
    pub fn with_mode(mut self, mode: SessionMode) -> Self {
        self.initial_mode = mode;
        self
    }

    /// Override the Chrome executable path.
    #[must_use]
    pub fn with_chrome_binary(mut self, binary: PathBuf) -> Self {
        self.chrome_binary = Some(binary);
        self
    }

    /// Configure media blocking mode (`None` disables media abort).
    #[must_use]
    pub fn with_media_block(mut self, mode: Option<MediaBlockMode>) -> Self {
        self.media_block = mode;
        self
    }

    /// Configure network URL capture patterns.
    #[must_use]
    pub fn with_capture_patterns(mut self, patterns: Vec<String>) -> Self {
        self.capture_url_patterns = patterns;
        self
    }
}

/// Interactive authentication policy passed to [`ChromeSession::ensure_authenticated`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPolicy {
    /// Fail immediately with `LOGIN_REQUIRED` or `VERIFICATION_REQUIRED` when unauthenticated.
    FailFast,
    /// Switch to headed takeover mode and wait up to `timeout` for human completion.
    InteractiveTakeover {
        /// Maximum time to wait for login/captcha completion.
        timeout: Duration,
        /// Polling interval while waiting for completion.
        poll_interval: Duration,
    },
}

/// High-level browser session orchestrating Chrome lifecycle, stealth CDP page operations,
/// Douyin authentication/takeover, in-page fetch, and network capture.
pub struct ChromeSession {
    config: SessionConfig,
    mode: SessionMode,
    fake_backend: bool,
    browser: Option<CdpBrowser>,
    page: Option<Arc<dyn CdpPageOps>>,
    capture_buffer: NetworkCaptureBuffer,
    started_at: Option<String>,
    context_generation: u64,
    mode_transition_count: u64,
    last_transition: Option<ModeTransition>,
    takeover: TakeoverState,
}

impl std::fmt::Debug for ChromeSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChromeSession")
            .field("profile_id", &self.config.paths.profile_id)
            .field("mode", &self.mode)
            .field("fake_backend", &self.fake_backend)
            .field("has_page", &self.page.is_some())
            .field("context_generation", &self.context_generation)
            .field("takeover", &self.takeover)
            .finish_non_exhaustive()
    }
}

impl ChromeSession {
    /// Create a new unstarted [`ChromeSession`].
    pub fn new(config: SessionConfig) -> Self {
        let mode = config.initial_mode;
        let fake_backend = mode == SessionMode::Fake;
        Self {
            config,
            mode,
            fake_backend,
            browser: None,
            page: None,
            capture_buffer: NetworkCaptureBuffer::new(),
            started_at: None,
            context_generation: 0,
            mode_transition_count: 0,
            last_transition: None,
            takeover: TakeoverState::default(),
        }
    }

    /// Create a [`ChromeSession`] pre-wired with an injected [`CdpPageOps`] backend (e.g. `FakeCdpPage`).
    pub fn with_page(paths: ProfilePaths, page: Arc<dyn CdpPageOps>, mode: SessionMode) -> Self {
        let config = SessionConfig::new(paths).with_mode(mode);
        let capture_buffer = page.capture_buffer();
        Self {
            config,
            mode,
            fake_backend: true,
            browser: None,
            page: Some(page),
            capture_buffer,
            started_at: Some(utc_now()),
            context_generation: 1,
            mode_transition_count: 0,
            last_transition: None,
            takeover: TakeoverState::default(),
        }
    }

    /// Return the resolved profile paths.
    pub fn paths(&self) -> &ProfilePaths {
        &self.config.paths
    }

    /// Return the current session mode.
    pub fn mode(&self) -> SessionMode {
        self.mode
    }

    /// Return the current interactive takeover state.
    pub fn takeover(&self) -> &TakeoverState {
        &self.takeover
    }

    /// Return the last headless/headed mode transition, if any.
    pub fn last_transition(&self) -> Option<&ModeTransition> {
        self.last_transition.as_ref()
    }

    /// Return the live page handle, launching Chrome and attaching a stealth page if needed.
    pub async fn ensure_page(&mut self) -> Result<Arc<dyn CdpPageOps>> {
        if let Some(page) = &self.page {
            return Ok(Arc::clone(page));
        }

        if self.mode == SessionMode::Fake {
            return Err(DoudouError::new(
                "BROKER_BROWSER_UNAVAILABLE",
                "Fake session was created without a FakeCdpPage backend.",
            ));
        }

        let mut options = ChromeLaunchOptions::new(
            self.config.paths.browser_data.clone(),
            self.mode.to_launch_mode(),
        )
        .with_startup_timeout(self.config.startup_timeout);
        if let Some(bin) = &self.config.chrome_binary {
            options = options.with_binary(bin.clone());
        }

        let browser = CdpBrowser::launch(options).await?;
        let page = browser.open_page().await?;
        if let Some(media_mode) = self.config.media_block {
            page.enable_media_abort(media_mode).await?;
        }
        let buffer = page
            .enable_network_capture(self.config.capture_url_patterns.clone())
            .await?;

        let page_arc: Arc<dyn CdpPageOps> = Arc::new(page);
        self.browser = Some(browser);
        self.page = Some(Arc::clone(&page_arc));
        self.capture_buffer = buffer;
        self.started_at = Some(utc_now());
        self.context_generation += 1;
        Ok(page_arc)
    }

    /// Switch between headless and headed Chrome modes while reusing the same `user_data_dir`.
    pub async fn switch_mode(
        &mut self,
        target: SessionMode,
        preserve_url: PreserveUrl,
    ) -> Result<ModeTransition> {
        let previous_mode = self.mode;
        let previous_url = if preserve_url == PreserveUrl::KeepDouyinUrl {
            match &self.page {
                Some(page) => page.current_url().await.ok(),
                None => None,
            }
        } else {
            None
        };

        if self.fake_backend {
            // In fake mode keep the injected FakeCdpPage so tests can verify takeover transitions.
            self.mode = target;
            self.mode_transition_count += 1;
            let transition = ModeTransition {
                at: utc_now(),
                from: previous_mode,
                to: target,
                page_url: previous_url,
            };
            self.last_transition = Some(transition.clone());
            return Ok(transition);
        }

        if previous_mode == target && self.page.is_some() {
            return Ok(ModeTransition {
                at: utc_now(),
                from: previous_mode,
                to: target,
                page_url: previous_url,
            });
        }

        // Tear down existing Chrome process before relaunching with the same profile directory.
        self.close_browser_only().await?;
        self.mode = target;
        let _ = self.ensure_page().await?;

        if let Some(url) = previous_url.as_deref()
            && url.starts_with(DOUYIN_WEB_ORIGIN)
        {
            let _ = self.navigate_douyin(url).await?;
        }

        self.mode_transition_count += 1;
        let current_url = match &self.page {
            Some(p) => p.current_url().await.ok(),
            None => None,
        };
        let transition = ModeTransition {
            at: utc_now(),
            from: previous_mode,
            to: target,
            page_url: current_url,
        };
        self.last_transition = Some(transition.clone());
        Ok(transition)
    }

    /// Navigate to a Douyin URL or relative route (e.g. `"/chat"` or `"https://www.douyin.com/chat"`).
    pub async fn navigate_douyin(&mut self, route_or_url: &str) -> Result<NavigationResult> {
        let target_url = resolve_douyin_url(route_or_url)?;
        let page = self.ensure_page().await?;
        let url = page
            .goto(
                &target_url,
                LoadWait::DomContentLoaded,
                Duration::from_secs(60),
            )
            .await?;
        let title = page.title().await.unwrap_or_default();
        Ok(NavigationResult { url, title })
    }

    /// Detect any visible login, captcha, slider, SMS, or passkey modal on the active page.
    pub async fn detect_verification_kind(&mut self) -> Result<Option<VerificationKind>> {
        let page = self.ensure_page().await?;
        detect_verification_on_page(page.as_ref()).await
    }

    /// Classify the current page's login state using cookies, IM capture, chat DOM, and modal checks.
    pub async fn classify_login(&mut self) -> Result<LoginClassification> {
        let page = self.ensure_page().await?;
        let cookies = page
            .cookies(&["https://www.douyin.com/", "https://imapi.douyin.com/"])
            .await
            .unwrap_or_default();
        let nonempty_cookie_count = cookies.iter().filter(|c| !c.value.is_empty()).count();
        let verification_kind = detect_verification_on_page(page.as_ref()).await?;
        let verification_visible = verification_kind.is_some();

        let im_initialized = self.captured_im_init().is_some();
        let chat_loaded = check_chat_loaded(page.as_ref()).await;
        let account_session_valid = im_initialized || chat_loaded;

        let (status, reason_code) = match verification_kind {
            Some(VerificationKind::QrLogin) => (
                LoginStatus::LoginRequired,
                Some("LOGIN_REQUIRED".to_owned()),
            ),
            Some(_) => (
                LoginStatus::VerificationRequired,
                Some("VERIFICATION_REQUIRED".to_owned()),
            ),
            None if account_session_valid => (LoginStatus::Valid, None),
            None => (
                LoginStatus::LoginRequired,
                Some("LOGIN_REQUIRED".to_owned()),
            ),
        };

        let valid = status == LoginStatus::Valid;
        let classification = LoginClassification {
            status,
            valid,
            reason_code,
            cookie_count: nonempty_cookie_count,
            authenticated_cookie_count: if valid { nonempty_cookie_count } else { 0 },
            im_initialized,
            chat_loaded,
            verification_visible,
            verification_kind,
        };
        self.record_login_state(&classification);
        Ok(classification)
    }

    /// Ensure the session is authenticated on Douyin.
    ///
    /// When login or verification is required:
    /// - Under [`AuthPolicy::FailFast`], returns `LOGIN_REQUIRED` or `VERIFICATION_REQUIRED`.
    /// - Under [`AuthPolicy::InteractiveTakeover`], switches to headed mode so the user can
    ///   complete login/captcha, waits until authenticated, and returns to headless mode.
    pub async fn ensure_authenticated(
        &mut self,
        policy: AuthPolicy,
    ) -> Result<LoginClassification> {
        let page = self.ensure_page().await?;
        let current = page.current_url().await.unwrap_or_default();
        if !current.starts_with(DOUYIN_WEB_ORIGIN) {
            let _ = self.navigate_douyin("/chat").await?;
        }

        let mut initial = self.classify_login().await?;
        if initial.valid {
            return Ok(initial);
        }

        // After navigating to /chat in a freshly launched headless Chrome, Douyin's SPA
        // needs a short hydration window to fire `get_by_user_init` and render chat DOM
        // even when valid auth cookies are already persisted in the profile.
        if !self.fake_backend
            && !initial.verification_visible
            && self.has_douyin_auth_cookie().await
        {
            let settle_deadline = Instant::now() + Duration::from_secs(12);
            while Instant::now() < settle_deadline {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let probe = self.classify_login().await?;
                if probe.valid {
                    return Ok(probe);
                }
                initial = probe;
                if initial.verification_visible {
                    break;
                }
            }
        }

        let AuthPolicy::InteractiveTakeover {
            timeout,
            poll_interval,
        } = policy
        else {
            let code = initial.reason_code.as_deref().unwrap_or("LOGIN_REQUIRED");
            return Err(DoudouError::new(
                code,
                "The Profile requires headed login or verification.",
            )
            .with_details(serde_json::to_value(&initial).unwrap_or(Value::Null)));
        };

        let previous_mode = self.mode;
        let reason = initial
            .reason_code
            .clone()
            .unwrap_or_else(|| "LOGIN_REQUIRED".to_owned());
        self.takeover = TakeoverState {
            active: true,
            reason: Some(reason),
            verification_kind: initial.verification_kind,
            started_at: Some(utc_now()),
        };

        info!(
            profile = %self.config.paths.profile_id,
            verification_kind = ?initial.verification_kind,
            "switching to headed takeover for interactive authentication"
        );
        self.switch_mode(SessionMode::HeadedTakeover, PreserveUrl::KeepDouyinUrl)
            .await?;

        let deadline = Instant::now() + timeout;
        let mut last_classification = initial;
        let mut observed_challenge = last_classification.verification_visible;
        let mut last_chat_nav: Option<Instant> = None;
        while Instant::now() < deadline {
            let classification = self.classify_login().await?;
            observed_challenge |= classification.verification_visible;
            if classification.valid {
                self.takeover = TakeoverState::default();
                let restore_mode = if previous_mode == SessionMode::Fake {
                    SessionMode::Fake
                } else {
                    SessionMode::Headless
                };
                let _ = self
                    .switch_mode(restore_mode, PreserveUrl::KeepDouyinUrl)
                    .await;
                return Ok(classification);
            }
            if !self.fake_backend && !classification.verification_visible {
                let current_url = match &self.page {
                    Some(page) => page.current_url().await.unwrap_or_default(),
                    None => String::new(),
                };
                let on_chat = current_url.starts_with("https://www.douyin.com/chat");
                let has_auth_cookie = self.has_douyin_auth_cookie().await;
                let can_navigate =
                    last_chat_nav.is_none_or(|prev| prev.elapsed() >= Duration::from_secs(8));
                if !on_chat && (observed_challenge || has_auth_cookie) && can_navigate {
                    last_chat_nav = Some(Instant::now());
                    let _ = self.navigate_douyin("/chat").await;
                }
            }
            last_classification = classification;
            tokio::time::sleep(poll_interval).await;
        }

        let code = match last_classification.verification_kind {
            None | Some(VerificationKind::QrLogin) => "LOGIN_TIMEOUT",
            Some(_) => "VERIFICATION_TIMEOUT",
        };
        Err(DoudouError::new(
            code,
            "Headed login or verification did not complete before the timeout.",
        )
        .with_details(serde_json::to_value(&last_classification).unwrap_or(Value::Null)))
    }

    /// Execute an in-page `fetch()` request on a validated Douyin origin.
    pub async fn page_fetch(&mut self, request: &FetchRequest) -> Result<FetchResponse> {
        validate_douyin_fetch_url(&request.url)?;
        let page = self.ensure_page().await?;
        let outer_timeout = Duration::from_millis(request.timeout_ms.saturating_add(2_000));
        match tokio::time::timeout(outer_timeout, page.fetch_in_page(request)).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(DoudouError::new(
                "PROTOCOL_TRANSPORT_TIMEOUT",
                "The page-context protocol request exceeded its bounded timeout.",
            )
            .with_details(json!({
                "url": request.url,
                "timeout_ms": request.timeout_ms,
            }))),
        }
    }

    /// Send a binary Protobuf `POST` request to `https://imapi.douyin.com/...` with bounded retries.
    pub async fn post_protobuf(&mut self, endpoint: &str, body: &[u8]) -> Result<(u16, Vec<u8>)> {
        if !endpoint.starts_with("https://imapi.douyin.com/") {
            return Err(DoudouError::new(
                "PROTOCOL_ENDPOINT_INVALID",
                "Only the supported IM protocol endpoint is allowed.",
            ));
        }

        let request = FetchRequest::post_bytes(endpoint, "application/x-protobuf", body);
        let attempts = PROTOBUF_RETRY_DELAYS.len() + 1;
        for attempt in 1..=attempts {
            match self.page_fetch(&request).await {
                Ok(resp) => {
                    let bytes = resp.decode_bytes()?;
                    return Ok((resp.status, bytes));
                }
                Err(err) => {
                    let transient = err.message.contains("NetworkError")
                        || err.message.contains("Failed to fetch");
                    if !transient || attempt == attempts {
                        if transient {
                            return Err(DoudouError::new(
                                "PROTOCOL_NETWORK_ERROR",
                                "The page-context IM protocol request failed after bounded retries.",
                            )
                            .with_details(json!({
                                "endpoint": endpoint,
                                "attempts": attempts,
                            })));
                        }
                        return Err(err);
                    }
                    debug!(
                        attempt,
                        endpoint, "retrying transient IM protobuf network error"
                    );
                    tokio::time::sleep(PROTOBUF_RETRY_DELAYS[attempt - 1]).await;
                }
            }
        }
        unreachable!("retry loop always returns on final attempt")
    }

    /// Return a snapshot of all captured network exchanges.
    pub fn captured_exchanges(&self) -> Vec<CapturedNetworkExchange> {
        self.capture_buffer.snapshot()
    }

    /// Drain and return all captured network exchanges.
    pub fn take_captured_exchanges(&self) -> Vec<CapturedNetworkExchange> {
        self.capture_buffer.take_all()
    }

    /// Access the shared [`NetworkCaptureBuffer`].
    pub fn capture_buffer(&self) -> NetworkCaptureBuffer {
        self.capture_buffer.clone()
    }

    /// Return the latest captured binary IM init `(request_body, response_body)` if valid Protobuf.
    pub fn captured_im_init(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        self.capture_buffer
            .snapshot()
            .into_iter()
            .rev()
            .find_map(|ex| {
                if !ex.url.contains(IM_INIT_PATH_MARKER) || ex.status != Some(200) {
                    return None;
                }
                let req = ex.request_body?;
                let resp = ex.response_body?;
                if resp.is_empty() || looks_like_json(&resp) {
                    return None;
                }
                Some((req, resp))
            })
    }

    /// Return all captured binary IM user-info response bodies.
    pub fn captured_user_info_responses(&self) -> Vec<Vec<u8>> {
        self.capture_buffer
            .snapshot()
            .into_iter()
            .filter(|ex| ex.url.contains(IM_USER_INFO_PATH_MARKER) && ex.status == Some(200))
            .filter_map(|ex| ex.response_body)
            .collect()
    }

    /// Collect current session telemetry.
    pub async fn status(&self) -> SessionStatus {
        let (page_url, blocked_media_requests) = match &self.page {
            Some(page) => (page.current_url().await.ok(), page.blocked_media_requests()),
            None => (None, 0),
        };
        SessionStatus {
            running: self.page.is_some(),
            mode: self.mode,
            started_at: self.started_at.clone(),
            page_url,
            blocked_media_requests,
            context_generation: self.context_generation,
            mode_transition_count: self.mode_transition_count,
            takeover: self.takeover.clone(),
            captured_exchange_count: self.capture_buffer.snapshot().len(),
        }
    }

    /// Close the active page and cleanly shut down the Chrome process.
    pub async fn close(&mut self) -> Result<()> {
        self.close_browser_only().await?;
        self.takeover = TakeoverState::default();
        Ok(())
    }

    async fn close_browser_only(&mut self) -> Result<()> {
        if let Some(page) = self.page.take() {
            let _ = page.close().await;
        }
        if let Some(mut browser) = self.browser.take() {
            browser.close().await?;
        }
        Ok(())
    }

    /// Return `true` when Douyin auth session cookies (`sessionid` / `sid_tt` / `uid_tt`) exist.
    pub async fn has_douyin_auth_cookie(&self) -> bool {
        let Some(page) = &self.page else {
            return false;
        };
        let Ok(cookies) = page
            .cookies(&["https://www.douyin.com/", "https://imapi.douyin.com/"])
            .await
        else {
            return false;
        };
        cookies.iter().any(|c| {
            !c.value.is_empty()
                && matches!(
                    c.name.as_str(),
                    "sessionid" | "sessionid_ss" | "sid_tt" | "uid_tt"
                )
        })
    }

    fn record_login_state(&self, login: &LoginClassification) {
        let Ok(Some(mut metadata)) = read_json(&self.config.paths.metadata, true) else {
            return;
        };
        let Some(obj) = metadata.as_object_mut() else {
            return;
        };
        let status_str = match login.status {
            LoginStatus::Valid => "valid",
            LoginStatus::LoginRequired => "login_required",
            LoginStatus::VerificationRequired => "verification_required",
        };
        obj.insert("login_status".to_owned(), json!(status_str));
        obj.insert(
            "last_error_code".to_owned(),
            login
                .reason_code
                .as_ref()
                .map_or(Value::Null, |code| json!(code)),
        );
        if login.valid {
            obj.insert("last_login_verified_at".to_owned(), json!(utc_now()));
        }
        obj.insert("updated_at".to_owned(), json!(utc_now()));
        let _ = atomic_write_json(&self.config.paths.metadata, &metadata, None);
    }
}

/// Controls whether [`ChromeSession::switch_mode`] re-navigates to the previous Douyin URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreserveUrl {
    /// Re-open the previous `https://www.douyin.com` URL after restarting Chrome.
    KeepDouyinUrl,
    /// Leave the restarted browser at `about:blank`.
    Blank,
}

fn resolve_douyin_url(route_or_url: &str) -> Result<String> {
    let trimmed = route_or_url.trim();
    if trimmed.is_empty() {
        return Ok(format!("{DOUYIN_WEB_ORIGIN}/"));
    }
    if trimmed.starts_with('/') {
        return Ok(format!("{DOUYIN_WEB_ORIGIN}{trimmed}"));
    }
    let parsed = url::Url::parse(trimmed).map_err(|err| {
        DoudouError::new(
            "INVALID_URL",
            format!("Invalid Douyin navigation URL '{trimmed}': {err}"),
        )
    })?;
    let host = parsed.host_str().unwrap_or_default();
    if parsed.scheme() != "https" || !(host == "www.douyin.com" || host.ends_with(".douyin.com")) {
        return Err(DoudouError::new(
            "INVALID_DOUYIN_ORIGIN",
            format!("Navigation target must be on https://*.douyin.com: {trimmed}"),
        ));
    }
    Ok(parsed.to_string())
}

fn validate_douyin_fetch_url(raw_url: &str) -> Result<()> {
    let parsed = url::Url::parse(raw_url).map_err(|err| {
        DoudouError::new(
            "PROTOCOL_ENDPOINT_INVALID",
            format!("Invalid protocol URL '{raw_url}': {err}"),
        )
    })?;
    let host = parsed.host_str().unwrap_or_default();
    if parsed.scheme() != "https" || !(host == "www.douyin.com" || host == "imapi.douyin.com") {
        return Err(DoudouError::new(
            "PROTOCOL_ENDPOINT_INVALID",
            "Only supported Douyin page-context protocol endpoints are allowed.",
        ));
    }
    Ok(())
}

fn looks_like_json(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .copied()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|first| matches!(first, b'{' | b'['))
}

/// Isolated-world DOM probe detecting Douyin auth/captcha/verification modals.
const VERIFICATION_DETECT_JS: &str = r#"(() => {
  const isVisible = (el) => {
    if (!el) return false;
    const style = window.getComputedStyle(el);
    if (!style || style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 1 && rect.height > 1;
  };
  const anyVisible = (selector) => Array.from(document.querySelectorAll(selector)).find(isVisible) || null;
  if (anyVisible('iframe[src*="captcha" i], [class*="captcha" i] [class*="slide" i], [class*="captcha" i] [class*="drag" i]')) {
    return 'slider';
  }
  const secondVerifySms = anyVisible('.second-verify-mask input');
  if (secondVerifySms) {
    return 'sms_code';
  }
  const qr = anyVisible('#douyin_login_comp_scan_code_qrcode img, [id*="qrcode" i] img, [class*="login" i] img[src*="qr" i], [class*="login" i] canvas[class*="qr" i], [class*="qrcode" i] img, [class*="qr-code" i] img');
  if (qr) {
    return 'qr_login';
  }
  const sms = anyVisible('input[placeholder*="验证码"], input[maxlength="6"]');
  if (sms) {
    const dialog = sms.closest('[role="dialog"], [class*="verify"], [class*="login"], #login-panel-new, #douyin_login_comp_flat_panel, .second-verify-mask') || document.body;
    const text = (dialog && dialog.innerText) || '';
    return text.includes('快捷登录') ? 'quick_login_sms' : 'sms_code';
  }
  const buttons = Array.from(document.querySelectorAll('[role="dialog"] button, [class*="login" i] button')).filter(isVisible);
  if (buttons.some((b) => /Passkey|通行密钥/i.test(b.innerText || ''))) {
    return 'passkey';
  }
  const generic = anyVisible('iframe[src*="verify" i], .second-verify-mask, [role="dialog"][class*="verify" i]');
  if (generic) {
    if (generic.tagName === 'IFRAME') return 'generic_verification';
    const text = (generic.innerText || '').toLowerCase();
    if (['验证', '验证码', '安全', '滑块', '拖动'].some((m) => text.includes(m))) {
      return 'generic_verification';
    }
  }
  return null;
})()"#;

const CHAT_LOADED_DETECT_JS: &str = r#"(() => {
  if (!window.location.href.startsWith('https://www.douyin.com/chat')) return false;
  return Boolean(
    document.querySelector('[data-e2e="conversation-item"]') ||
    document.querySelector('[role="listitem"]') ||
    document.querySelector('[contenteditable="true"]')
  );
})()"#;

async fn detect_verification_on_page(page: &dyn CdpPageOps) -> Result<Option<VerificationKind>> {
    let val = page
        .eval_json(VERIFICATION_DETECT_JS, EvalWorld::Isolated)
        .await?;
    Ok(val.as_str().and_then(VerificationKind::from_tag))
}

async fn check_chat_loaded(page: &dyn CdpPageOps) -> bool {
    page.eval_json(CHAT_LOADED_DETECT_JS, EvalWorld::Isolated)
        .await
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}
