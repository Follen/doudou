//! Loopback TCP JSON-line broker server with single-flight serialization, auth token,
//! profile locking, idle timeout, and strict at-most-once write semantics.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use doudou_cdp::{AuthPolicy, ChromeSession, PreserveUrl, SessionConfig, SessionMode};
use doudou_core::{
    DoudouError, ProfilePaths, Result, absolutize_path, atomic_write_json, atomic_write_text,
    read_json, utc_now,
};
use doudou_storage::load_profile;
use rand::RngCore as _;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex as AsyncMutex, Notify};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::broker::protocol::{
    BROKER_DESCRIPTOR_SCHEMA, BrokerDescriptor, BrokerEndpoint, CommandSemantics,
    DEFAULT_COMMAND_TIMEOUT_SECONDS, DETERMINISTIC_WRITE_ERRORS, MAX_COMMAND_TIMEOUT_SECONDS,
    MAX_FRAME_BYTES, MIN_COMMAND_TIMEOUT_SECONDS, constant_time_eq, read_bounded_line,
};
use crate::broker::registry::{register_broker, unregister_broker};
use crate::lock::ProfileLock;
use crate::process::current_process_identity;

const MAX_CACHED_REQUEST_RESULTS: usize = 256;

/// Trait implemented by business command routers plugged into [`BrokerServer`].
#[async_trait]
pub trait BrokerHandler: Send + Sync {
    /// Execute a business command against `session`. Return `Ok(None)` if the command is unhandled.
    async fn handle(
        &self,
        command: &str,
        params: &Value,
        session: &Arc<AsyncMutex<ChromeSession>>,
        home: &Path,
        data_root: &Path,
        profile_name: &str,
    ) -> Result<Option<Value>>;
}

/// Closure-backed [`BrokerHandler`] helper for tests and lightweight custom brokers.
pub struct FnBrokerHandler<F, Fut> {
    func: F,
    _marker: std::marker::PhantomData<fn() -> Fut>,
}

impl<F, Fut> FnBrokerHandler<F, Fut>
where
    F: Send
        + Sync
        + Fn(String, Value, Arc<AsyncMutex<ChromeSession>>, PathBuf, PathBuf, String) -> Fut,
    Fut: Future<Output = Result<Option<Value>>> + Send + 'static,
{
    /// Wrap an async closure into a [`BrokerHandler`].
    pub fn new(func: F) -> Self {
        Self {
            func,
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<F, Fut> BrokerHandler for FnBrokerHandler<F, Fut>
where
    F: Send
        + Sync
        + Fn(String, Value, Arc<AsyncMutex<ChromeSession>>, PathBuf, PathBuf, String) -> Fut,
    Fut: Future<Output = Result<Option<Value>>> + Send + 'static,
{
    async fn handle(
        &self,
        command: &str,
        params: &Value,
        session: &Arc<AsyncMutex<ChromeSession>>,
        home: &Path,
        data_root: &Path,
        profile_name: &str,
    ) -> Result<Option<Value>> {
        (self.func)(
            command.to_owned(),
            params.clone(),
            Arc::clone(session),
            home.to_path_buf(),
            data_root.to_path_buf(),
            profile_name.to_owned(),
        )
        .await
    }
}

/// Configuration for launching a [`BrokerServer`].
#[derive(Debug, Clone)]
pub struct BrokerServerConfig {
    /// User-level Doudou home directory (`doudou_home`).
    pub home: PathBuf,
    /// Project-local business data root.
    pub data_root: PathBuf,
    /// Target profile name.
    pub profile_name: String,
    /// Idle duration after which the broker exits automatically.
    pub idle_timeout: Duration,
    /// Whether to run with a fake in-memory browser session rather than launching Chrome.
    pub fake_browser: bool,
    /// Optional explicit Chrome executable override.
    pub chrome_binary: Option<PathBuf>,
    /// Maximum time to wait during interactive takeover recovery on reads.
    pub takeover_timeout: Duration,
}

impl BrokerServerConfig {
    /// Construct a default configuration with a 1200-second idle timeout.
    pub fn new(home: PathBuf, data_root: PathBuf, profile_name: impl Into<String>) -> Self {
        Self {
            home,
            data_root,
            profile_name: profile_name.into(),
            idle_timeout: Duration::from_secs(1200),
            fake_browser: false,
            chrome_binary: None,
            takeover_timeout: Duration::from_secs(600),
        }
    }

    /// Override the idle timeout duration.
    #[must_use]
    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Enable or disable fake browser mode.
    #[must_use]
    pub fn with_fake_browser(mut self, fake: bool) -> Self {
        self.fake_browser = fake;
        self
    }

    /// Override the Chrome binary path.
    #[must_use]
    pub fn with_chrome_binary(mut self, binary: PathBuf) -> Self {
        self.chrome_binary = Some(binary);
        self
    }

    /// Override the interactive takeover timeout for read recovery.
    #[must_use]
    pub fn with_takeover_timeout(mut self, timeout: Duration) -> Self {
        self.takeover_timeout = timeout;
        self
    }
}

#[derive(Debug, Default)]
struct ResultCache {
    entries: HashMap<String, Value>,
    order: VecDeque<String>,
}

impl ResultCache {
    fn get(&self, key: &str) -> Option<Value> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: String, value: Value) {
        if !self.entries.contains_key(&key) {
            self.order.push_back(key.clone());
        }
        self.entries.insert(key, value);
        while self.entries.len() > MAX_CACHED_REQUEST_RESULTS {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

struct BrokerState {
    home: PathBuf,
    data_root: PathBuf,
    profile_name: String,
    paths: ProfilePaths,
    idle_timeout: Duration,
    takeover_timeout: Duration,
    token: String,
    session: Arc<AsyncMutex<ChromeSession>>,
    handler: Option<Arc<dyn BrokerHandler>>,
    /// Single-flight mutex serializing all broker commands per profile (`NoConcurrentWrites`).
    command_lock: Arc<AsyncMutex<()>>,
    queue_count: AtomicUsize,
    active_request: Mutex<Option<String>>,
    last_completed_monotonic: Mutex<Instant>,
    last_completed_at: Mutex<String>,
    request_results: Mutex<ResultCache>,
    stopped: std::sync::atomic::AtomicBool,
    stop_notify: Notify,
    registration_path: Mutex<Option<PathBuf>>,
    profile_lock: Mutex<Option<ProfileLock>>,
}

impl BrokerState {
    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    fn trigger_stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.stop_notify.notify_waiters();
    }

    async fn refresh_descriptor(&self) {
        let Ok(Some(mut desc)) = read_json(&self.paths.broker_descriptor, true) else {
            return;
        };
        let pid = std::process::id();
        if desc.get("pid").and_then(Value::as_u64) != Some(u64::from(pid)) {
            return;
        }
        let status = {
            let guard = self.session.lock().await;
            guard.status().await
        };
        let last_activity = self
            .last_completed_at
            .lock()
            .expect("last_completed_at lock poisoned")
            .clone();
        if let Some(obj) = desc.as_object_mut() {
            obj.insert("last_activity_at".to_owned(), json!(last_activity));
            obj.insert("mode".to_owned(), json!(session_mode_label(status.mode)));
            obj.insert(
                "takeover".to_owned(),
                json!({
                    "active": status.takeover.active,
                    "verification_kind": status.takeover.verification_kind,
                }),
            );
            let _ = atomic_write_json(&self.paths.broker_descriptor, &desc, Some(0o600));
        }
    }

    async fn cleanup(&self) {
        {
            let mut guard = self.session.lock().await;
            let _ = guard.close().await;
        }
        let _ = fs::remove_file(&self.paths.broker_descriptor);
        let _ = fs::remove_file(&self.paths.broker_token);
        let reg = self
            .registration_path
            .lock()
            .expect("registration lock poisoned")
            .take();
        unregister_broker(reg.as_deref());
        let lock = self
            .profile_lock
            .lock()
            .expect("profile lock mutex poisoned")
            .take();
        if let Some(mut held) = lock {
            held.release();
        }
    }

    /// Execute one already-authenticated request JSON value under the single-flight command lock.
    async fn execute(&self, request: &Value) -> Value {
        let request_id = request
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let command = request
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let semantics_raw = request
            .get("semantics")
            .and_then(Value::as_str)
            .unwrap_or("read");
        let Some(semantics) = CommandSemantics::parse(semantics_raw) else {
            return error_response(
                None,
                &DoudouError::new(
                    "BROKER_REQUEST_INVALID",
                    "request_id, command and valid semantics are required.",
                ),
            );
        };
        if request_id.is_empty() || command.is_empty() {
            return error_response(
                None,
                &DoudouError::new(
                    "BROKER_REQUEST_INVALID",
                    "request_id, command and valid semantics are required.",
                ),
            );
        }

        // Idempotent replay check: return cached result immediately without re-executing.
        if let Some(cached) = self
            .request_results
            .lock()
            .expect("result cache poisoned")
            .get(&request_id)
        {
            return cached;
        }

        self.queue_count.fetch_add(1, Ordering::SeqCst);
        let queue_guard = QueueDecrementGuard(&self.queue_count);
        let _cmd_guard = self.command_lock.lock().await;
        // Decrement queue count as soon as we hold the command lock.
        drop(queue_guard);

        // Re-check cache after acquiring single-flight lock in case an identical request_id
        // completed while we were waiting in the queue!
        if let Some(cached) = self
            .request_results
            .lock()
            .expect("result cache poisoned")
            .get(&request_id)
        {
            return cached;
        }

        let active_guard = ActiveRequestGuard::new(&self.active_request, request_id.clone());

        let started_at = utc_now();
        let requested_timeout = request
            .get("command_timeout_seconds")
            .and_then(Value::as_f64)
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT_SECONDS);
        let command_timeout =
            requested_timeout.clamp(MIN_COMMAND_TIMEOUT_SECONDS, MAX_COMMAND_TIMEOUT_SECONDS);
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

        let outcome = tokio::time::timeout(
            Duration::from_secs_f64(command_timeout),
            self.dispatch_with_read_recovery(&command, &params, semantics),
        )
        .await;

        let response = match outcome {
            Ok(Ok(result)) => {
                let resp = json!({
                    "ok": true,
                    "request_id": request_id,
                    "semantics": semantics.as_str(),
                    "started_at": started_at,
                    "completed_at": utc_now(),
                    "result": result,
                });
                self.request_results
                    .lock()
                    .expect("result cache poisoned")
                    .insert(request_id.clone(), resp.clone());
                resp
            }
            Err(_) => {
                let code = if semantics == CommandSemantics::Write {
                    "WRITE_RESULT_UNKNOWN"
                } else {
                    "BROKER_COMMAND_TIMEOUT"
                };
                let err = DoudouError::new(
                    code,
                    "The broker command exceeded its total time limit; writes are not replayed automatically.",
                )
                .with_details(json!({
                    "request_id": request_id,
                    "command": command,
                    "timeout_seconds": command_timeout,
                    "write_started": semantics == CommandSemantics::Write,
                }));
                let resp = error_response(Some(&request_id), &err);
                if semantics == CommandSemantics::Write {
                    self.request_results
                        .lock()
                        .expect("result cache poisoned")
                        .insert(request_id.clone(), resp.clone());
                }
                resp
            }
            Ok(Err(err)) => {
                self.classify_command_error(&request_id, semantics, err)
                    .await
            }
        };

        drop(active_guard);
        {
            *self
                .last_completed_monotonic
                .lock()
                .expect("last_completed_monotonic lock poisoned") = Instant::now();
            *self
                .last_completed_at
                .lock()
                .expect("last_completed_at lock poisoned") = utc_now();
        }
        self.refresh_descriptor().await;
        response
    }

    async fn classify_command_error(
        &self,
        request_id: &str,
        semantics: CommandSemantics,
        err: DoudouError,
    ) -> Value {
        if matches!(
            err.code.as_str(),
            "LOGIN_REQUIRED" | "VERIFICATION_REQUIRED" | "WRITE_VERIFICATION_REQUIRED"
        ) {
            let mut details = err
                .details
                .as_ref()
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let kind = {
                let mut guard = self.session.lock().await;
                guard.detect_verification_kind().await.ok().flatten()
            };
            if err.code == "LOGIN_REQUIRED" || kind.is_some() {
                details.insert(
                    "takeover".to_owned(),
                    json!({
                        "active": true,
                        "reason": err.code,
                        "verification_kind": kind,
                    }),
                );
            } else {
                details.insert(
                    "takeover".to_owned(),
                    json!({
                        "active": false,
                        "reason": err.code,
                        "verification_kind": Value::Null,
                    }),
                );
            }
            let enriched =
                DoudouError::new(err.code, err.message).with_details(Value::Object(details));
            let resp = error_response(Some(request_id), &enriched);
            if semantics == CommandSemantics::Write {
                self.request_results
                    .lock()
                    .expect("result cache poisoned")
                    .insert(request_id.to_owned(), resp.clone());
            }
            return resp;
        }

        let write_started = err.is_write_started();

        if DETERMINISTIC_WRITE_ERRORS.contains(&err.code.as_str()) || !write_started {
            let resp = error_response(Some(request_id), &err);
            self.request_results
                .lock()
                .expect("result cache poisoned")
                .insert(request_id.to_owned(), resp.clone());
            return resp;
        }

        if semantics == CommandSemantics::Write {
            let wrapped = DoudouError::new(
                "WRITE_RESULT_UNKNOWN",
                "The write started but a complete receipt was not obtained; it will not be replayed automatically.",
            )
            .with_details(json!({
                "request_id": request_id,
                "write_started": true,
                "cause_code": err.code,
                "cause_details": err.details,
            }));
            let resp = error_response(Some(request_id), &wrapped);
            self.request_results
                .lock()
                .expect("result cache poisoned")
                .insert(request_id.to_owned(), resp.clone());
            return resp;
        }

        error_response(Some(request_id), &err)
    }

    async fn dispatch_with_read_recovery(
        &self,
        command: &str,
        params: &Value,
        semantics: CommandSemantics,
    ) -> Result<Value> {
        match self.dispatch(command, params).await {
            Ok(val) => Ok(val),
            Err(err) => {
                if semantics != CommandSemantics::Read
                    || !matches!(
                        err.code.as_str(),
                        "LOGIN_REQUIRED" | "VERIFICATION_REQUIRED"
                    )
                {
                    return Err(err);
                }
                let kind = {
                    let mut guard = self.session.lock().await;
                    guard.detect_verification_kind().await.ok().flatten()
                };
                if err.code == "VERIFICATION_REQUIRED" && kind.is_none() {
                    return Err(err);
                }
                {
                    let mut guard = self.session.lock().await;
                    let _ = guard
                        .ensure_authenticated(AuthPolicy::InteractiveTakeover {
                            timeout: self.takeover_timeout,
                            poll_interval: Duration::from_millis(500),
                        })
                        .await?;
                }
                self.dispatch(command, params).await
            }
        }
    }

    async fn dispatch(&self, command: &str, params: &Value) -> Result<Value> {
        match command {
            "ping" => Ok(json!({
                "pong": true,
                "pid": std::process::id(),
            })),
            "status" => {
                let browser = {
                    let guard = self.session.lock().await;
                    guard.status().await
                };
                let active = self
                    .active_request
                    .lock()
                    .expect("active_request lock poisoned")
                    .clone();
                let last_completed = self
                    .last_completed_at
                    .lock()
                    .expect("last_completed_at lock poisoned")
                    .clone();
                Ok(json!({
                    "pid": std::process::id(),
                    "profile_id": self.paths.profile_id,
                    "data_root": self.data_root.display().to_string(),
                    "queue_count": self.queue_count.load(Ordering::SeqCst),
                    "active_request": active,
                    "idle_timeout_seconds": self.idle_timeout.as_secs_f64(),
                    "last_completed_at": last_completed,
                    "browser": browser,
                }))
            }
            "browser.visible" => {
                let visible = params
                    .get("visible")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let target = if visible {
                    SessionMode::HeadedTakeover
                } else {
                    SessionMode::Headless
                };
                let mut guard = self.session.lock().await;
                let transition = guard
                    .switch_mode(target, PreserveUrl::KeepDouyinUrl)
                    .await?;
                let status = guard.status().await;
                Ok(json!({
                    "visible": visible,
                    "mode": session_mode_label(status.mode),
                    "transition": transition,
                    "status": status,
                }))
            }
            "page.open" => {
                let url = params.get("url").and_then(Value::as_str).unwrap_or("");
                if !url.starts_with("https://www.douyin.com/") && url != "https://www.douyin.com" {
                    return Err(DoudouError::new(
                        "BROKER_URL_NOT_ALLOWED",
                        "The broker only opens supported Douyin pages.",
                    ));
                }
                let mut guard = self.session.lock().await;
                let nav = guard.navigate_douyin(url).await?;
                Ok(serde_json::to_value(nav).unwrap_or(Value::Null))
            }
            "login.wait" => {
                let timeout_secs = params
                    .get("timeout_seconds")
                    .and_then(Value::as_f64)
                    .unwrap_or(600.0)
                    .clamp(0.1, MAX_COMMAND_TIMEOUT_SECONDS);
                let mut guard = self.session.lock().await;
                let auth = guard
                    .ensure_authenticated(AuthPolicy::InteractiveTakeover {
                        timeout: Duration::from_secs_f64(timeout_secs),
                        poll_interval: Duration::from_millis(250),
                    })
                    .await?;
                Ok(json!({
                    "status": "ready",
                    "login": auth,
                }))
            }
            "shutdown" => {
                self.trigger_stop();
                Ok(json!({ "stopping": true }))
            }
            _ => {
                if let Some(handler) = &self.handler
                    && let Some(handled) = handler
                        .handle(
                            command,
                            params,
                            &self.session,
                            &self.home,
                            &self.data_root,
                            &self.profile_name,
                        )
                        .await?
                {
                    return Ok(handled);
                }
                match command {
                    "session.status" => {
                        let status = {
                            let guard = self.session.lock().await;
                            guard.status().await
                        };
                        Ok(json!({
                            "status": "ok",
                            "browser": status,
                        }))
                    }
                    "session.refresh" => {
                        let mut guard = self.session.lock().await;
                        let login = guard.ensure_authenticated(AuthPolicy::FailFast).await?;
                        let status = guard.status().await;
                        Ok(json!({
                            "status": "ready",
                            "initialization_count": status.context_generation,
                            "page_url": status.page_url,
                            "login": login,
                        }))
                    }
                    _ => Err(DoudouError::new(
                        "BROKER_COMMAND_UNKNOWN",
                        format!("Unknown broker command: {command}."),
                    )),
                }
            }
        }
    }
}

struct QueueDecrementGuard<'a>(&'a AtomicUsize);

impl Drop for QueueDecrementGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct ActiveRequestGuard<'a>(&'a Mutex<Option<String>>);

impl<'a> ActiveRequestGuard<'a> {
    fn new(slot: &'a Mutex<Option<String>>, request_id: String) -> Self {
        *slot.lock().expect("active_request lock poisoned") = Some(request_id);
        Self(slot)
    }
}

impl Drop for ActiveRequestGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = None;
        }
    }
}

fn error_response(request_id: Option<&str>, err: &DoudouError) -> Value {
    match request_id {
        Some(id) => json!({
            "ok": false,
            "request_id": id,
            "error": err.as_event(),
        }),
        None => json!({
            "ok": false,
            "error": err.as_event(),
        }),
    }
}

fn generate_token() -> String {
    let mut bytes = [0u8; 48];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn session_mode_label(mode: SessionMode) -> &'static str {
    match mode {
        SessionMode::Headless => "headless",
        SessionMode::HeadedTakeover => "headed_takeover",
        SessionMode::Fake => "fake",
    }
}

async fn handle_tcp_connection(state: Arc<BrokerState>, stream: TcpStream) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let response = match read_bounded_line(&mut reader, MAX_FRAME_BYTES).await {
        Ok(None) => error_response(
            None,
            &DoudouError::new("BROKER_REQUEST_INVALID", "The broker request is empty."),
        ),
        Ok(Some(line)) => match serde_json::from_slice::<Value>(&line) {
            Ok(request) => {
                let supplied = request
                    .get("token")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if constant_time_eq(supplied, &state.token) {
                    state.execute(&request).await
                } else {
                    error_response(
                        None,
                        &DoudouError::new("BROKER_AUTH_FAILED", "Broker authentication failed."),
                    )
                }
            }
            Err(_) => error_response(
                None,
                &DoudouError::new(
                    "BROKER_REQUEST_INVALID",
                    "The broker request is not valid JSON.",
                ),
            ),
        },
        Err(err) => error_response(None, &err),
    };

    if let Ok(mut encoded) = serde_json::to_vec(&response) {
        encoded.push(b'\n');
        let _ = write_half.write_all(&encoded).await;
        let _ = write_half.flush().await;
    }
    let _ = write_half.shutdown().await;
}

/// Running loopback TCP broker server handle.
pub struct BrokerServer {
    state: Arc<BrokerState>,
    descriptor: BrokerDescriptor,
    driver_task: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for BrokerServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerServer")
            .field("profile_id", &self.descriptor.profile_id)
            .field("endpoint", &self.descriptor.endpoint)
            .finish_non_exhaustive()
    }
}

impl BrokerServer {
    /// Start a broker server for `config` with a default [`ChromeSession`].
    pub async fn start(
        config: BrokerServerConfig,
        handler: Option<Arc<dyn BrokerHandler>>,
    ) -> Result<Self> {
        let (_meta, paths) = load_profile(&config.home, &config.data_root, &config.profile_name)?;
        let session = if config.fake_browser {
            doudou_cdp::FakeBrowserSession::authenticated(paths).into_chrome_session()
        } else {
            let mut sc = SessionConfig::new(paths).with_mode(SessionMode::Headless);
            if let Some(bin) = &config.chrome_binary {
                sc = sc.with_chrome_binary(bin.clone());
            }
            ChromeSession::new(sc)
        };
        Self::start_with_session(config, session, handler).await
    }

    /// Start a broker server backed by an explicitly constructed [`ChromeSession`] (e.g. fake page).
    pub async fn start_with_session(
        config: BrokerServerConfig,
        session: ChromeSession,
        handler: Option<Arc<dyn BrokerHandler>>,
    ) -> Result<Self> {
        let data_root = absolutize_path(&config.data_root);
        let home = absolutize_path(&config.home);
        let (_meta, paths) = load_profile(&home, &data_root, &config.profile_name)?;

        let mut lock = ProfileLock::new(
            paths.lock.clone(),
            paths.profile_id.clone(),
            data_root.clone(),
            "broker",
        );
        lock.acquire()?;

        let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(l) => l,
            Err(err) => {
                lock.release();
                return Err(DoudouError::new(
                    "BROKER_BIND_FAILED",
                    format!("Failed binding broker loopback TCP socket: {err}"),
                ));
            }
        };
        let local_addr = listener.local_addr().map_err(|err| {
            lock.release();
            DoudouError::new(
                "BROKER_BIND_FAILED",
                format!("Failed reading bound broker address: {err}"),
            )
        })?;

        let token = generate_token();
        let identity = current_process_identity();
        let started_at = utc_now();
        let mode_label = session_mode_label(session.mode()).to_owned();

        let endpoint = BrokerEndpoint {
            transport: "tcp".to_owned(),
            host: "127.0.0.1".to_owned(),
            port: local_addr.port(),
        };
        let descriptor = BrokerDescriptor {
            schema: BROKER_DESCRIPTOR_SCHEMA.to_owned(),
            pid: identity.pid,
            process_start: identity.start,
            data_root: data_root.display().to_string(),
            profile_id: paths.profile_id.clone(),
            endpoint,
            token_file: paths
                .broker_token
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("broker.token")
                .to_owned(),
            mode: mode_label,
            browser_version: None,
            created_at: started_at.clone(),
            last_activity_at: started_at.clone(),
            takeover: None,
        };

        if let Err(err) = fs::create_dir_all(&paths.runtime) {
            lock.release();
            return Err(DoudouError::new(
                "IO_ERROR",
                format!("Failed creating runtime directory: {err}"),
            ));
        }
        if let Err(err) = atomic_write_text(&paths.broker_token, &format!("{token}\n"), Some(0o600))
        {
            lock.release();
            return Err(err);
        }
        if let Err(err) = atomic_write_json(&paths.broker_descriptor, &descriptor, Some(0o600)) {
            let _ = fs::remove_file(&paths.broker_token);
            lock.release();
            return Err(err);
        }
        let reg_path = match register_broker(&paths, &descriptor) {
            Ok(path) => Some(path),
            Err(err) => {
                let _ = fs::remove_file(&paths.broker_descriptor);
                let _ = fs::remove_file(&paths.broker_token);
                lock.release();
                return Err(err);
            }
        };

        let state = Arc::new(BrokerState {
            home,
            data_root,
            profile_name: config.profile_name,
            paths,
            idle_timeout: config.idle_timeout,
            takeover_timeout: config.takeover_timeout,
            token,
            session: Arc::new(AsyncMutex::new(session)),
            handler,
            command_lock: Arc::new(AsyncMutex::new(())),
            queue_count: AtomicUsize::new(0),
            active_request: Mutex::new(None),
            last_completed_monotonic: Mutex::new(Instant::now()),
            last_completed_at: Mutex::new(started_at),
            request_results: Mutex::new(ResultCache::default()),
            stopped: std::sync::atomic::AtomicBool::new(false),
            stop_notify: Notify::new(),
            registration_path: Mutex::new(reg_path),
            profile_lock: Mutex::new(Some(lock)),
        });

        let driver_state = Arc::clone(&state);
        let driver_task = tokio::spawn(async move {
            run_broker_loop(driver_state, listener).await;
        });

        info!(
            profile = %descriptor.profile_id,
            port = descriptor.endpoint.port,
            "broker server started"
        );

        Ok(Self {
            state,
            descriptor,
            driver_task: Mutex::new(Some(driver_task)),
        })
    }

    /// Return the broker descriptor written to `broker.json`.
    pub fn descriptor(&self) -> &BrokerDescriptor {
        &self.descriptor
    }

    /// Return the broker loopback TCP endpoint.
    pub fn endpoint(&self) -> &BrokerEndpoint {
        &self.descriptor.endpoint
    }

    /// Return the per-broker authentication token.
    pub fn token(&self) -> &str {
        &self.state.token
    }

    /// Return `true` when the broker has been signalled to stop.
    pub fn is_stopped(&self) -> bool {
        self.state.is_stopped()
    }

    /// Return `true` when a command currently holds the single-flight lock.
    pub fn is_command_locked(&self) -> bool {
        self.state.command_lock.try_lock().is_err()
    }

    /// Return the currently executing `request_id`, if any.
    pub fn active_request(&self) -> Option<String> {
        self.state
            .active_request
            .lock()
            .expect("active_request lock poisoned")
            .clone()
    }

    /// Access the shared browser session mutex.
    pub fn session(&self) -> &Arc<AsyncMutex<ChromeSession>> {
        &self.state.session
    }

    /// Execute an in-process authenticated broker request directly (bypassing TCP framing).
    pub async fn execute(&self, request: &Value) -> Value {
        self.state.execute(request).await
    }

    /// Signal the broker to stop gracefully.
    pub fn request_shutdown(&self) {
        self.state.trigger_stop();
    }

    /// Wait until the broker loop exits (due to idle timeout or shutdown) and cleanup finishes.
    pub async fn wait_stopped(&self) {
        let handle = self
            .driver_task
            .lock()
            .expect("driver_task lock poisoned")
            .take();
        if let Some(task) = handle {
            let _ = task.await;
        }
    }

    /// Gracefully shut down the broker and wait until all runtime files and locks are released.
    pub async fn close(&self) {
        self.state.trigger_stop();
        self.wait_stopped().await;
    }
}

impl Drop for BrokerServer {
    fn drop(&mut self) {
        self.state.trigger_stop();
    }
}

async fn run_broker_loop(state: Arc<BrokerState>, listener: TcpListener) {
    let idle_interval =
        Duration::from_secs_f64((state.idle_timeout.as_secs_f64() / 10.0).clamp(0.02, 5.0));
    let mut idle_tick = tokio::time::interval(idle_interval);
    // Skip the immediate first tick so idle timeout counts from after startup.
    idle_tick.tick().await;

    let mut connection_tasks: Vec<JoinHandle<()>> = Vec::new();

    loop {
        if state.is_stopped() {
            break;
        }
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _peer)) => {
                        let conn_state = Arc::clone(&state);
                        connection_tasks.push(tokio::spawn(async move {
                            handle_tcp_connection(conn_state, stream).await;
                        }));
                    }
                    Err(err) => {
                        warn!(error = %err, "broker accept error");
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                }
            }
            _ = idle_tick.tick() => {
                let locked = state.command_lock.try_lock().is_err();
                let queued = state.queue_count.load(Ordering::SeqCst);
                let idle_for = state
                    .last_completed_monotonic
                    .lock()
                    .expect("last_completed_monotonic lock poisoned")
                    .elapsed();
                if !locked && queued == 0 && idle_for >= state.idle_timeout {
                    debug!(
                        profile = %state.paths.profile_id,
                        idle_ms = idle_for.as_millis(),
                        "broker idle timeout reached; shutting down"
                    );
                    state.trigger_stop();
                    break;
                }
            }
            () = state.stop_notify.notified() => {
                if state.is_stopped() {
                    break;
                }
            }
        }
        connection_tasks.retain(|handle| !handle.is_finished());
    }

    // Drain active client handlers briefly before tearing down resources.
    for task in connection_tasks {
        let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
    }
    state.cleanup().await;
}
