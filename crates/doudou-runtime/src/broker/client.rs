//! Async TCP broker client with 16 MiB frame limit enforcement and strict write non-replay.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use doudou_core::{DoudouError, ProfilePaths, Result, absolutize_path, random_id, read_json};
use doudou_storage::load_profile;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt as _, BufReader};
use tokio::net::TcpStream;

use crate::broker::protocol::{
    CommandSemantics, MAX_FRAME_BYTES, command_timeout_seconds, read_bounded_line,
};
use crate::broker::registry::descriptor_is_live;
use crate::process::{ProcessIdentity, process_identity, same_process};

/// Re-exported response frame byte limit (16 MiB) matching Python's `BROKER_RESPONSE_LIMIT_BYTES`.
pub const BROKER_RESPONSE_LIMIT_BYTES: usize = MAX_FRAME_BYTES;

/// Configuration for spawning a detached background broker executable on demand.
#[derive(Debug, Clone)]
pub struct BrokerLauncher {
    /// Executable path to spawn (defaults to current binary).
    pub executable: PathBuf,
    /// Arguments placed before `--home`, `--data-root`, `--profile`, `--idle-timeout`.
    pub prefix_args: Vec<String>,
}

impl BrokerLauncher {
    /// Build a launcher targeting the current executable with subcommand arguments (e.g. `["broker", "serve"]`).
    pub fn current_exe(prefix_args: Vec<String>) -> Result<Self> {
        let executable = std::env::current_exe().map_err(|err| {
            DoudouError::new(
                "IO_ERROR",
                format!("Failed resolving current executable path: {err}"),
            )
        })?;
        Ok(Self {
            executable,
            prefix_args,
        })
    }
}

/// Client communicating with a per-profile loopback TCP [`super::BrokerServer`].
pub struct BrokerClient {
    home: PathBuf,
    data_root: PathBuf,
    profile_name: String,
    paths: ProfilePaths,
    start_timeout: Duration,
    fake_browser: bool,
    launcher: Option<BrokerLauncher>,
    last_process_identity: Mutex<Option<ProcessIdentity>>,
    launcher_process_identity: Mutex<Option<ProcessIdentity>>,
}

impl std::fmt::Debug for BrokerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerClient")
            .field("profile_id", &self.paths.profile_id)
            .field("data_root", &self.data_root)
            .field("fake_browser", &self.fake_browser)
            .finish_non_exhaustive()
    }
}

impl BrokerClient {
    /// Construct a client for an existing profile under `(home, data_root)`.
    pub fn new(home: &Path, data_root: &Path, profile_name: &str) -> Result<Self> {
        let home = absolutize_path(home);
        let data_root = absolutize_path(data_root);
        let (_meta, paths) = load_profile(&home, &data_root, profile_name)?;
        Ok(Self {
            home,
            data_root,
            profile_name: profile_name.to_owned(),
            paths,
            start_timeout: Duration::from_secs(90),
            fake_browser: false,
            launcher: None,
            last_process_identity: Mutex::new(None),
            launcher_process_identity: Mutex::new(None),
        })
    }

    /// Override the maximum time spent waiting for a newly spawned broker to publish its descriptor.
    #[must_use]
    pub fn with_start_timeout(mut self, timeout: Duration) -> Self {
        self.start_timeout = timeout;
        self
    }

    /// Enable fake-browser mode when spawning a broker subprocess.
    #[must_use]
    pub fn with_fake_browser(mut self, fake: bool) -> Self {
        self.fake_browser = fake;
        self
    }

    /// Provide an explicit [`BrokerLauncher`] for auto-starting detached broker processes.
    #[must_use]
    pub fn with_launcher(mut self, launcher: BrokerLauncher) -> Self {
        self.launcher = Some(launcher);
        self
    }

    /// Return the resolved profile paths.
    pub fn paths(&self) -> &ProfilePaths {
        &self.paths
    }

    /// Read and validate the live broker descriptor for this profile, pruning stale files.
    pub fn descriptor(&self) -> Option<Value> {
        let descriptor = read_json(&self.paths.broker_descriptor, true)
            .ok()
            .flatten()?;
        if descriptor_is_live(&descriptor, &self.data_root, &self.paths.profile_id) {
            if !self.paths.broker_token.is_file() {
                return None;
            }
            let pid = descriptor
                .get("pid")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(0);
            let start = descriptor
                .get("process_start")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            *self
                .last_process_identity
                .lock()
                .expect("last_process_identity lock poisoned") =
                Some(ProcessIdentity::new(pid, start, None));
            return Some(descriptor);
        }
        let _ = fs::remove_file(&self.paths.broker_descriptor);
        let _ = fs::remove_file(&self.paths.broker_token);
        None
    }

    /// Read the per-broker secret token with a short retry window while startup settles.
    pub async fn token(&self) -> Result<String> {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if let Ok(raw) = fs::read_to_string(&self.paths.broker_token) {
                let trimmed = raw.trim().to_owned();
                if !trimmed.is_empty() {
                    return Ok(trimmed);
                }
            }
            let current = read_json(&self.paths.broker_descriptor, true)
                .ok()
                .flatten();
            let still_live = current
                .as_ref()
                .is_some_and(|d| descriptor_is_live(d, &self.data_root, &self.paths.profile_id));
            if !still_live {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Err(DoudouError::new(
            "BROKER_TOKEN_MISSING",
            format!(
                "Broker token file not available: {}",
                self.paths.broker_token.display()
            ),
        ))
    }

    /// Ensure a live broker is running for this profile, spawning one if configured and absent.
    pub async fn ensure_started(&self) -> Result<Value> {
        if let Some(desc) = self.descriptor() {
            return Ok(desc);
        }
        if let Some(current) = read_json(&self.paths.broker_descriptor, true)
            .ok()
            .flatten()
            && descriptor_is_live(&current, &self.data_root, &self.paths.profile_id)
        {
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                if let Some(desc) = self.descriptor() {
                    return Ok(desc);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        let launcher = match &self.launcher {
            Some(l) => l.clone(),
            None => BrokerLauncher::current_exe(vec!["broker".to_owned(), "serve".to_owned()])?,
        };

        fs::create_dir_all(&self.paths.runtime).map_err(|err| {
            DoudouError::new(
                "IO_ERROR",
                format!("Failed creating broker runtime dir: {err}"),
            )
        })?;
        let idle_seconds =
            std::env::var("DOUDOU_BROKER_IDLE_SECONDS").unwrap_or_else(|_| "1200".to_owned());

        let build_cmd = |extra_flags: u32| -> Result<Command> {
            let stdout = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.paths.broker_stdout)
                .map_err(|err| {
                    DoudouError::new(
                        "IO_ERROR",
                        format!("Failed opening broker stdout log: {err}"),
                    )
                })?;
            let stderr = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.paths.broker_stderr)
                .map_err(|err| {
                    DoudouError::new(
                        "IO_ERROR",
                        format!("Failed opening broker stderr log: {err}"),
                    )
                })?;

            let mut cmd = Command::new(&launcher.executable);
            cmd.args(&launcher.prefix_args)
                .args([
                    "--home",
                    &self.home.display().to_string(),
                    "--data-root",
                    &self.data_root.display().to_string(),
                    "--profile",
                    &self.profile_name,
                    "--idle-timeout",
                    &idle_seconds,
                ])
                .current_dir(&self.data_root)
                .stdin(Stdio::null())
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr));
            if self.fake_browser {
                cmd.arg("--fake-browser");
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt as _;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
                cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | extra_flags);
            }
            #[cfg(not(windows))]
            let _ = extra_flags;
            Ok(cmd)
        };

        #[cfg(windows)]
        let child = {
            const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
            match build_cmd(CREATE_BREAKAWAY_FROM_JOB)?.spawn() {
                Ok(child) => child,
                Err(err) if err.raw_os_error() == Some(5) => {
                    build_cmd(0)?.spawn().map_err(|fallback_err| {
                        DoudouError::new(
                            "BROKER_START_FAILED",
                            format!("Failed spawning broker process: {fallback_err}"),
                        )
                    })?
                }
                Err(err) => {
                    return Err(DoudouError::new(
                        "BROKER_START_FAILED",
                        format!("Failed spawning broker process: {err}"),
                    ));
                }
            }
        };

        #[cfg(not(windows))]
        let child = build_cmd(0)?.spawn().map_err(|err| {
            DoudouError::new(
                "BROKER_START_FAILED",
                format!("Failed spawning broker process: {err}"),
            )
        })?;
        *self
            .launcher_process_identity
            .lock()
            .expect("launcher_process_identity lock poisoned") = process_identity(child.id());

        let deadline = Instant::now() + self.start_timeout;
        while Instant::now() < deadline {
            if let Some(desc) = self.descriptor() {
                return Ok(desc);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let tail = fs::read_to_string(&self.paths.broker_stderr)
            .map(|s| {
                let start = s.len().saturating_sub(4000);
                s[start..].to_owned()
            })
            .unwrap_or_default();
        Err(DoudouError::new(
            "BROKER_START_TIMEOUT",
            "The Profile broker did not become ready before the timeout.",
        )
        .with_details(json!({ "stderr_tail": tail })))
    }

    /// Send one RPC request to this profile's broker (auto-starting it when `start` is `true`).
    pub async fn request_async(
        &self,
        command: &str,
        params: Value,
        semantics: CommandSemantics,
        request_id: Option<&str>,
        start: bool,
    ) -> Result<Value> {
        let descriptor = if start {
            self.ensure_started().await?
        } else {
            self.descriptor().ok_or_else(|| {
                DoudouError::new("BROKER_NOT_RUNNING", "The Profile broker is not running.")
            })?
        };

        let host = descriptor
            .pointer("/endpoint/host")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                DoudouError::new(
                    "BROKER_DESCRIPTOR_INVALID",
                    "Broker descriptor missing host.",
                )
            })?;
        let port = descriptor
            .pointer("/endpoint/port")
            .and_then(Value::as_u64)
            .and_then(|v| u16::try_from(v).ok())
            .ok_or_else(|| {
                DoudouError::new(
                    "BROKER_DESCRIPTOR_INVALID",
                    "Broker descriptor missing port.",
                )
            })?;
        let token = self.token().await?;

        Self::send_to_endpoint_with_data_root(
            host,
            port,
            &token,
            command,
            params,
            semantics,
            request_id,
            Some(&self.data_root),
        )
        .await
    }

    /// Send a single framed JSON-line request directly to `(host, port)` with `token`.
    ///
    /// Enforces [`BROKER_RESPONSE_LIMIT_BYTES`] (16 MiB) and never replays write requests.
    pub async fn send_to_endpoint(
        host: &str,
        port: u16,
        token: &str,
        command: &str,
        params: Value,
        semantics: CommandSemantics,
        request_id: Option<&str>,
    ) -> Result<Value> {
        Self::send_to_endpoint_with_data_root(
            host, port, token, command, params, semantics, request_id, None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_to_endpoint_with_data_root(
        host: &str,
        port: u16,
        token: &str,
        command: &str,
        params: Value,
        semantics: CommandSemantics,
        request_id: Option<&str>,
        data_root: Option<&Path>,
    ) -> Result<Value> {
        let req_id = request_id.map_or_else(|| random_id("request"), ToOwned::to_owned);
        let timeout_secs = command_timeout_seconds(command, &params);
        let mut payload = json!({
            "token": token,
            "request_id": req_id,
            "command": command,
            "semantics": semantics.as_str(),
            "params": params,
            "command_timeout_seconds": timeout_secs,
        });
        if let Some(root) = data_root
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert("data_root".to_owned(), json!(root.display().to_string()));
        }

        let stream = TcpStream::connect((host, port)).await.map_err(|err| {
            if semantics == CommandSemantics::Write {
                DoudouError::new(
                    "WRITE_RESULT_UNKNOWN",
                    "The broker connection failed after a write may have started; the request was not replayed.",
                )
                .with_details(json!({ "request_id": req_id, "write_started": true }))
            } else {
                DoudouError::new(
                    "BROKER_CONNECT_FAILED",
                    format!("Failed connecting to broker at {host}:{port}: {err}"),
                )
            }
        })?;

        let (read_half, mut write_half) = stream.into_split();
        let mut encoded = serde_json::to_vec(&payload).map_err(|err| {
            DoudouError::new(
                "JSON_SERIALIZE_ERROR",
                format!("Failed serializing broker request: {err}"),
            )
        })?;
        encoded.push(b'\n');

        write_half.write_all(&encoded).await.map_err(|err| {
            if semantics == CommandSemantics::Write {
                DoudouError::new(
                    "WRITE_RESULT_UNKNOWN",
                    "The broker write connection failed; the request was not replayed.",
                )
                .with_details(json!({ "request_id": req_id, "write_started": true }))
            } else {
                DoudouError::new(
                    "BROKER_IO_ERROR",
                    format!("Failed writing broker request: {err}"),
                )
            }
        })?;
        let _ = write_half.flush().await;

        let mut reader = BufReader::new(read_half);
        let read_timeout = Duration::from_secs_f64(timeout_secs + 5.0);
        let read_result = tokio::time::timeout(
            read_timeout,
            read_bounded_line(&mut reader, BROKER_RESPONSE_LIMIT_BYTES),
        )
        .await;

        // Always shut down the write half cleanly so the server sees EOF.
        let _ = write_half.shutdown().await;

        let line = match read_result {
            Err(_) => {
                if semantics == CommandSemantics::Write {
                    return Err(DoudouError::new(
                        "WRITE_RESULT_UNKNOWN",
                        "The broker response timed out after a write may have started; it was not replayed.",
                    )
                    .with_details(json!({
                        "request_id": req_id,
                        "timeout_seconds": timeout_secs,
                        "write_started": true,
                    })));
                }
                return Err(DoudouError::new(
                    "BROKER_COMMAND_TIMEOUT",
                    "The broker response exceeded its total time limit.",
                )
                .with_details(json!({
                    "request_id": req_id,
                    "timeout_seconds": timeout_secs,
                })));
            }
            Ok(Err(err)) if err.code == "BROKER_FRAME_TOO_LARGE" => {
                if semantics == CommandSemantics::Write {
                    return Err(DoudouError::new(
                        "WRITE_RESULT_UNKNOWN",
                        "The broker response exceeded the client limit after the write started; it was not replayed.",
                    )
                    .with_details(json!({
                        "request_id": req_id,
                        "response_limit_bytes": BROKER_RESPONSE_LIMIT_BYTES,
                        "write_started": true,
                    })));
                }
                return Err(DoudouError::new(
                    "BROKER_RESPONSE_TOO_LARGE",
                    "The broker response exceeded the client limit.",
                )
                .with_details(json!({
                    "request_id": req_id,
                    "response_limit_bytes": BROKER_RESPONSE_LIMIT_BYTES,
                })));
            }
            Ok(Err(err)) => return Err(err),
            Ok(Ok(None)) => {
                if semantics == CommandSemantics::Write {
                    return Err(DoudouError::new(
                        "WRITE_RESULT_UNKNOWN",
                        "The broker disconnected after the write started; the request was not replayed.",
                    )
                    .with_details(json!({ "request_id": req_id, "write_started": true })));
                }
                return Err(DoudouError::new(
                    "BROKER_DISCONNECTED",
                    "The broker disconnected before returning a response.",
                ));
            }
            Ok(Ok(Some(bytes))) => bytes,
        };

        let response: Value = serde_json::from_slice(&line).map_err(|err| {
            DoudouError::new(
                "BROKER_RESPONSE_INVALID",
                format!("Broker returned invalid JSON response: {err}"),
            )
        })?;

        if !response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let error = response.get("error").unwrap_or(&Value::Null);
            let code = error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("BROKER_REQUEST_FAILED");
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Broker request failed.");
            let mut out = DoudouError::new(code, message);
            if let Some(details) = error.get("details").filter(|v| !v.is_null()) {
                out = out.with_details(details.clone());
            }
            return Err(out);
        }

        Ok(response)
    }

    /// Wait until the broker descriptor, lock file, and observed OS process have all exited.
    pub async fn wait_stopped(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let last_stopped = {
                let expected = self
                    .last_process_identity
                    .lock()
                    .expect("last_process_identity lock poisoned")
                    .clone();
                match expected {
                    // If the broker ran in-process (same PID as the caller), judge by descriptor + lock cleanup.
                    Some(exp) if exp.pid == std::process::id() => true,
                    Some(exp) => !same_process(Some(&exp), process_identity(exp.pid).as_ref()),
                    None => true,
                }
            };
            let launcher_stopped = {
                let expected = self
                    .launcher_process_identity
                    .lock()
                    .expect("launcher_process_identity lock poisoned")
                    .clone();
                match expected {
                    Some(exp) if exp.pid == std::process::id() => true,
                    Some(exp) => !same_process(Some(&exp), process_identity(exp.pid).as_ref()),
                    None => true,
                }
            };
            if !self.paths.broker_descriptor.exists()
                && !self.paths.lock.exists()
                && last_stopped
                && launcher_stopped
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }
}
