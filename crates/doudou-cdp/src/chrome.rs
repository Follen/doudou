//! Cross-platform Chrome/Chromium/Edge discovery, version probing, and stealth launch.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use doudou_core::{DoudouError, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::{Child, Command};
use tracing::{debug, warn};

/// Environment variable overriding Chrome/Chromium/Edge executable discovery.
pub const CHROME_BIN_ENV: &str = "DOUDOU_CHROME_BIN";

/// Default startup timeout while waiting for `DevToolsActivePort` or `/json/version`.
pub const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(20);

/// Default browser UI locale flag.
pub const DEFAULT_LANG: &str = "zh-CN";

/// Default browser window dimensions `(width, height)`.
pub const DEFAULT_WINDOW_SIZE: (u32, u32) = (1280, 900);

/// Forbidden launch flag that exposes automation to anti-bot detectors.
const FORBIDDEN_AUTOMATION_FLAG: &str = "--enable-automation";

/// Supported Chromium-family browser brands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BrowserBrand {
    /// Google Chrome.
    Chrome,
    /// Open-source Chromium.
    Chromium,
    /// Microsoft Edge (Chromium-based).
    Edge,
    /// User-supplied custom Chromium binary.
    Custom,
}

/// How a browser executable was located on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DiscoverySource {
    /// Configured via `DOUDOU_CHROME_BIN` or explicit option.
    Explicit,
    /// Located from Windows Registry `App Paths`.
    WindowsRegistry,
    /// Located in a platform-standard installation directory.
    StandardPath,
    /// Located by scanning `PATH`.
    PathEnv,
}

/// Discovered Chromium-family executable and metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChromeBinary {
    /// Absolute or resolved filesystem path to the browser executable.
    pub path: PathBuf,
    /// Detected browser brand.
    pub brand: BrowserBrand,
    /// Best-effort dotted version string (e.g. `131.0.6778.85`).
    pub version: Option<String>,
    /// Where this executable was discovered.
    pub source: DiscoverySource,
}

/// Browser window presentation mode for Chrome launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    /// Invisible background mode using `--headless=new`.
    Headless,
    /// Visible desktop window for interactive login/captcha takeover.
    Headed,
}

/// Configuration for launching a stealth Chrome instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChromeLaunchOptions {
    /// Optional explicit path to Chrome/Chromium/Edge; auto-discovered when `None`.
    pub binary: Option<PathBuf>,
    /// Persistent profile user-data directory (`--user-data-dir`).
    pub user_data_dir: PathBuf,
    /// Headless or headed launch mode.
    pub mode: LaunchMode,
    /// UI language flag (`--lang`).
    pub lang: String,
    /// Optional initial window size `(width, height)`.
    pub window_size: Option<(u32, u32)>,
    /// Debugging port (`0` asks Chrome to bind an ephemeral localhost port).
    pub debugging_port: u16,
    /// Maximum time to wait for the CDP WebSocket endpoint to become ready.
    pub startup_timeout: Duration,
    /// Additional Chrome CLI flags (`--enable-automation` is always rejected).
    pub extra_args: Vec<String>,
}

impl ChromeLaunchOptions {
    /// Create launch options with Doudou stealth defaults for a profile directory.
    pub fn new(user_data_dir: PathBuf, mode: LaunchMode) -> Self {
        Self {
            binary: None,
            user_data_dir,
            mode,
            lang: DEFAULT_LANG.to_owned(),
            window_size: Some(DEFAULT_WINDOW_SIZE),
            debugging_port: 0,
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            extra_args: Vec::new(),
        }
    }

    /// Override the Chrome binary path.
    #[must_use]
    pub fn with_binary(mut self, binary: PathBuf) -> Self {
        self.binary = Some(binary);
        self
    }

    /// Override the `--lang` flag value.
    #[must_use]
    pub fn with_lang(mut self, lang: impl Into<String>) -> Self {
        self.lang = lang.into();
        self
    }

    /// Override the remote debugging port (`0` selects an ephemeral port).
    #[must_use]
    pub fn with_debugging_port(mut self, port: u16) -> Self {
        self.debugging_port = port;
        self
    }

    /// Override the startup readiness timeout.
    #[must_use]
    pub fn with_startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    /// Append an extra Chrome CLI argument.
    #[must_use]
    pub fn with_extra_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }
}

/// Active Chrome `DevTools` Protocol WebSocket endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevToolsEndpoint {
    /// Local TCP port Chrome bound for `DevTools`.
    pub port: u16,
    /// Full browser-level WebSocket debugger URL.
    pub ws_url: String,
    /// Optional `Browser` header reported by `/json/version`.
    pub browser_version: Option<String>,
}

/// Running Chrome child process with automatic cleanup on drop.
#[derive(Debug)]
pub struct ChromeProcess {
    child: Option<Child>,
    pid: Option<u32>,
    binary: ChromeBinary,
    user_data_dir: PathBuf,
    mode: LaunchMode,
    endpoint: DevToolsEndpoint,
}

impl ChromeProcess {
    /// Launch a stealth Chrome process and wait until its CDP endpoint is ready.
    pub async fn launch(options: ChromeLaunchOptions) -> Result<Self> {
        let binary = match &options.binary {
            Some(path) => validate_explicit_binary(path)?,
            None => discover_chrome()?,
        };

        fs::create_dir_all(&options.user_data_dir).map_err(|err| {
            DoudouError::new(
                "IO_ERROR",
                format!(
                    "Failed creating Chrome user-data directory {}: {err}",
                    options.user_data_dir.display()
                ),
            )
        })?;

        // Remove any stale DevToolsActivePort file left behind by an earlier crash.
        let port_file = options.user_data_dir.join("DevToolsActivePort");
        if port_file.exists() {
            let _ = fs::remove_file(&port_file);
        }

        let args = build_launch_args(&options)?;
        debug!(
            binary = %binary.path.display(),
            mode = ?options.mode,
            user_data_dir = %options.user_data_dir.display(),
            "launching Chrome process"
        );

        let mut cmd = Command::new(&binary.path);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            if options.mode == LaunchMode::Headless {
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
        }

        let mut child = cmd.spawn().map_err(|err| {
            DoudouError::new(
                "CHROME_LAUNCH_FAILED",
                format!(
                    "Failed to spawn Chrome binary {}: {err}",
                    binary.path.display()
                ),
            )
            .with_details(json!({
                "binary": binary.path.display().to_string(),
                "mode": options.mode,
            }))
        })?;

        let pid = child.id();
        let endpoint = match wait_for_devtools_endpoint(
            &mut child,
            &port_file,
            options.debugging_port,
            options.startup_timeout,
        )
        .await
        {
            Ok(ep) => ep,
            Err(err) => {
                let _ = child.kill().await;
                return Err(err);
            }
        };

        Ok(Self {
            child: Some(child),
            pid,
            binary,
            user_data_dir: options.user_data_dir,
            mode: options.mode,
            endpoint,
        })
    }

    /// Return the discovered browser binary metadata.
    pub fn binary(&self) -> &ChromeBinary {
        &self.binary
    }

    /// Return the resolved `DevTools` WebSocket endpoint.
    pub fn endpoint(&self) -> &DevToolsEndpoint {
        &self.endpoint
    }

    /// Return the OS process identifier if known.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Return the active launch mode.
    pub fn mode(&self) -> LaunchMode {
        self.mode
    }

    /// Return the user-data directory bound to this Chrome process.
    pub fn user_data_dir(&self) -> &Path {
        &self.user_data_dir
    }

    /// Check whether the Chrome child process is still running.
    pub fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Terminate the Chrome child process cleanly and wait for exit.
    pub async fn close(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            if let Ok(Some(_)) = child.try_wait() {
                return Ok(());
            }
            if let Err(err) = child.kill().await
                && err.kind() != std::io::ErrorKind::InvalidInput
            {
                warn!(error = %err, "failed to kill Chrome child process");
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
        }
        Ok(())
    }
}

impl Drop for ChromeProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }
}

/// Build stealth Chrome CLI arguments from [`ChromeLaunchOptions`].
///
/// Rejects `--enable-automation` in `extra_args` so automation flags can never leak.
pub fn build_launch_args(options: &ChromeLaunchOptions) -> Result<Vec<String>> {
    for arg in &options.extra_args {
        if arg.eq_ignore_ascii_case(FORBIDDEN_AUTOMATION_FLAG)
            || arg
                .to_ascii_lowercase()
                .starts_with(&format!("{FORBIDDEN_AUTOMATION_FLAG}="))
        {
            return Err(DoudouError::new(
                "CHROME_STEALTH_VIOLATION",
                "The --enable-automation flag is forbidden in stealth mode.",
            ));
        }
    }

    let mut args = vec![
        format!("--user-data-dir={}", options.user_data_dir.display()),
        format!("--remote-debugging-port={}", options.debugging_port),
        "--disable-blink-features=AutomationControlled".to_owned(),
        "--no-first-run".to_owned(),
        "--no-default-browser-check".to_owned(),
        "--disable-infobars".to_owned(),
        format!("--lang={}", options.lang),
        "--mute-audio".to_owned(),
    ];

    if let Some((width, height)) = options.window_size {
        args.push(format!("--window-size={width},{height}"));
    }

    if options.mode == LaunchMode::Headless {
        args.push("--headless=new".to_owned());
    }

    args.extend(options.extra_args.iter().cloned());
    args.push("about:blank".to_owned());
    Ok(args)
}

/// Discover the best available Chrome, Chromium, or Edge binary on this machine.
///
/// Checks `DOUDOU_CHROME_BIN` first, then platform-specific locations and `PATH`.
pub fn discover_chrome() -> Result<ChromeBinary> {
    let env_bin = std::env::var(CHROME_BIN_ENV).ok();
    discover_chrome_with(env_bin.as_deref(), &[])
}

/// Discover all installed Chrome/Chromium/Edge binaries in priority order.
pub fn discover_all_chromes() -> Vec<ChromeBinary> {
    let env_bin = std::env::var(CHROME_BIN_ENV).ok();
    collect_chrome_candidates(env_bin.as_deref(), &[])
}

/// Discover a Chrome binary using an optional explicit env override and extra candidate paths.
pub fn discover_chrome_with(
    env_bin: Option<&str>,
    extra_candidates: &[PathBuf],
) -> Result<ChromeBinary> {
    if let Some(raw) = env_bin.map(str::trim).filter(|s| !s.is_empty()) {
        return validate_explicit_binary(Path::new(raw));
    }
    collect_chrome_candidates(None, extra_candidates)
        .into_iter()
        .next()
        .ok_or_else(|| {
            DoudouError::new(
                "CHROME_NOT_FOUND",
                "No Chrome, Chromium, or Microsoft Edge executable was found. Set DOUDOU_CHROME_BIN to specify one.",
            )
        })
}

fn validate_explicit_binary(path: &Path) -> Result<ChromeBinary> {
    if !path.is_file() {
        return Err(DoudouError::new(
            "CHROME_NOT_FOUND",
            format!(
                "Configured Chrome binary does not exist: {}",
                path.display()
            ),
        )
        .with_details(json!({ "path": path.display().to_string() })));
    }
    Ok(ChromeBinary {
        brand: infer_brand(path),
        version: probe_chrome_version(path),
        path: path.to_path_buf(),
        source: DiscoverySource::Explicit,
    })
}

fn collect_chrome_candidates(
    env_bin: Option<&str>,
    extra_candidates: &[PathBuf],
) -> Vec<ChromeBinary> {
    let mut raw_candidates: Vec<(PathBuf, DiscoverySource)> = Vec::new();

    if let Some(raw) = env_bin.map(str::trim).filter(|s| !s.is_empty()) {
        raw_candidates.push((PathBuf::from(raw), DiscoverySource::Explicit));
    }

    for extra in extra_candidates {
        raw_candidates.push((extra.clone(), DiscoverySource::StandardPath));
    }

    raw_candidates.extend(platform_candidates());

    let mut seen = Vec::new();
    let mut found = Vec::new();
    for (path, source) in raw_candidates {
        if !path.is_file() {
            continue;
        }
        let canonical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.iter().any(|existing| existing == &canonical) {
            continue;
        }
        seen.push(canonical);
        let brand = infer_brand(&path);
        let version = probe_chrome_version(&path);
        found.push(ChromeBinary {
            path,
            brand,
            version,
            source,
        });
    }
    found
}

/// Infer the [`BrowserBrand`] from a binary path's filename.
pub fn infer_brand(path: &Path) -> BrowserBrand {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.contains("edge") || name.contains("msedge") {
        BrowserBrand::Edge
    } else if name.contains("chromium") {
        BrowserBrand::Chromium
    } else if name.contains("chrome") {
        BrowserBrand::Chrome
    } else {
        BrowserBrand::Custom
    }
}

#[cfg(windows)]
fn platform_candidates() -> Vec<(PathBuf, DiscoverySource)> {
    let mut out = Vec::new();

    // 1. Windows Registry App Paths (Chrome first, then Edge).
    for exe in ["chrome.exe", "msedge.exe"] {
        for hive in [
            format!(r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}"),
            format!(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}"),
            format!(r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\{exe}"),
        ] {
            if let Some(path) = query_windows_registry_default(&hive) {
                out.push((path, DiscoverySource::WindowsRegistry));
            }
        }
    }

    // 2. Standard Program Files / LocalAppData paths.
    let roots: Vec<PathBuf> = [
        std::env::var_os("PROGRAMFILES").map(PathBuf::from),
        std::env::var_os("PROGRAMFILES(X86)").map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    .collect();

    let suffixes = [
        r"Google\Chrome\Application\chrome.exe",
        r"Google\Chrome Beta\Application\chrome.exe",
        r"Google\Chrome SxS\Application\chrome.exe",
        r"Chromium\Application\chrome.exe",
        r"Microsoft\Edge\Application\msedge.exe",
        r"Microsoft\Edge Beta\Application\msedge.exe",
    ];
    for suffix in suffixes {
        for root in &roots {
            out.push((root.join(suffix), DiscoverySource::StandardPath));
        }
    }

    // 3. PATH scan.
    for name in ["chrome.exe", "chromium.exe", "msedge.exe"] {
        if let Some(found) = find_on_path(name) {
            out.push((found, DiscoverySource::PathEnv));
        }
    }

    out
}

#[cfg(target_os = "macos")]
fn platform_candidates() -> Vec<(PathBuf, DiscoverySource)> {
    let mut out = vec![
        (
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            DiscoverySource::StandardPath,
        ),
        (
            PathBuf::from(
                "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            ),
            DiscoverySource::StandardPath,
        ),
        (
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
            DiscoverySource::StandardPath,
        ),
        (
            PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
            DiscoverySource::StandardPath,
        ),
    ];

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for rel in [
            "Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "Applications/Chromium.app/Contents/MacOS/Chromium",
            "Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ] {
            out.push((home.join(rel), DiscoverySource::StandardPath));
        }
    }

    for name in ["google-chrome", "chromium", "microsoft-edge"] {
        if let Some(found) = find_on_path(name) {
            out.push((found, DiscoverySource::PathEnv));
        }
    }

    out
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn platform_candidates() -> Vec<(PathBuf, DiscoverySource)> {
    let mut out = Vec::new();
    let names = [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "microsoft-edge",
        "microsoft-edge-stable",
    ];

    for name in names {
        if let Some(found) = find_on_path(name) {
            out.push((found, DiscoverySource::PathEnv));
        }
    }

    let standard = [
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/microsoft-edge",
        "/usr/bin/microsoft-edge-stable",
        "/snap/bin/chromium",
        "/usr/local/bin/google-chrome",
        "/usr/local/bin/chromium",
    ];
    for candidate in standard {
        out.push((PathBuf::from(candidate), DiscoverySource::StandardPath));
    }

    out
}

fn find_on_path(executable: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(executable);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(windows)]
fn query_windows_registry_default(key: &str) -> Option<PathBuf> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let output = std::process::Command::new("reg")
        .args(["query", key, "/ve"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_reg_query_default_value(&stdout).map(PathBuf::from)
}

/// Parse the default `(Default)` string value from Windows `reg query <key> /ve` output.
pub fn parse_reg_query_default_value(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let trimmed = line.trim();
        let Some(pos) = trimmed
            .find("REG_SZ")
            .or_else(|| trimmed.find("REG_EXPAND_SZ"))
        else {
            continue;
        };
        let after_type = &trimmed[pos..];
        let mut parts = after_type.splitn(2, char::is_whitespace);
        let _reg_type = parts.next();
        if let Some(value) = parts.next() {
            let cleaned = value.trim().trim_matches('"');
            if !cleaned.is_empty() {
                return Some(cleaned.to_owned());
            }
        }
    }
    None
}

/// Probe the dotted version string of a Chrome/Chromium/Edge binary without hanging.
///
/// On Windows, inspects PE `VS_FIXEDFILEINFO` metadata and sibling version directories first
/// (avoiding spawning GUI Chrome), falling back to `--version` on Unix platforms.
pub fn probe_chrome_version(path: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        if let Some(version) = probe_pe_version_from_file(path) {
            return Some(version);
        }
        if let Some(version) = probe_sibling_version_dir(path) {
            return Some(version);
        }
        None
    }

    #[cfg(not(windows))]
    {
        probe_version_via_cli(path, Duration::from_secs(3))
    }
}

#[cfg(not(windows))]
fn probe_version_via_cli(path: &Path, timeout: Duration) -> Option<String> {
    let mut child = std::process::Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            let output = child.wait_with_output().ok()?;
            let combined = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return parse_version_string(&combined);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn probe_pe_version_from_file(path: &Path) -> Option<String> {
    // Chrome/Edge launcher executables are small (~2-4 MiB); cap read at 16 MiB for safety.
    let meta = fs::metadata(path).ok()?;
    if meta.len() > 16 * 1024 * 1024 {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    parse_pe_fixed_file_info(&bytes)
}

#[cfg(windows)]
fn probe_sibling_version_dir(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    let entries = fs::read_dir(parent).ok()?;
    let mut versions: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_dotted_quad_version(&name) {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    versions.sort();
    versions.pop()
}

/// Extract the `Major.Minor.Build.Revision` version from a Windows PE `VS_FIXEDFILEINFO` block.
///
/// Prefers the real `.rsrc` `VS_VERSION_INFO` header (UTF-16LE) immediately preceding the
/// little-endian `VS_FIXEDFILEINFO` signature (`0xFEEF04BD`) so code constants in `.text`
/// are never misread as version numbers. Falls back to standalone `VS_FIXEDFILEINFO` blocks
/// for minimal synthetic buffers.
pub fn parse_pe_fixed_file_info(bytes: &[u8]) -> Option<String> {
    const SIGNATURE: [u8; 4] = [0xBD, 0x04, 0xEF, 0xFE];
    // UTF-16LE encoding of "VS_VERSION_INFO\0" (32 bytes).
    const VS_VERSION_INFO_UTF16: [u8; 32] = [
        b'V', 0, b'S', 0, b'_', 0, b'V', 0, b'E', 0, b'R', 0, b'S', 0, b'I', 0, b'O', 0, b'N', 0,
        b'_', 0, b'I', 0, b'N', 0, b'F', 0, b'O', 0, 0, 0,
    ];

    fn decode_at(bytes: &[u8], offset: usize) -> Option<String> {
        if offset + 52 > bytes.len() || bytes[offset..offset + 4] != SIGNATURE {
            return None;
        }
        let struct_version = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        if struct_version != 0x0001_0000 {
            return None;
        }
        let version_high = u32::from_le_bytes([
            bytes[offset + 8],
            bytes[offset + 9],
            bytes[offset + 10],
            bytes[offset + 11],
        ]);
        let version_low = u32::from_le_bytes([
            bytes[offset + 12],
            bytes[offset + 13],
            bytes[offset + 14],
            bytes[offset + 15],
        ]);
        let major = (version_high >> 16) & 0xFFFF;
        let minor = version_high & 0xFFFF;
        let build = (version_low >> 16) & 0xFFFF;
        let revision = version_low & 0xFFFF;
        if major > 0 {
            Some(format!("{major}.{minor}.{build}.{revision}"))
        } else {
            None
        }
    }

    if bytes.len() < 52 {
        return None;
    }

    // Pass 1: anchor on UTF-16LE "VS_VERSION_INFO\0" followed by 0..=4 alignment bytes.
    if bytes.len() >= VS_VERSION_INFO_UTF16.len() + 52 {
        for pos in 0..=bytes.len() - VS_VERSION_INFO_UTF16.len() - 52 {
            if bytes[pos..pos + VS_VERSION_INFO_UTF16.len()] != VS_VERSION_INFO_UTF16 {
                continue;
            }
            let base = pos + VS_VERSION_INFO_UTF16.len();
            for pad in 0..=4 {
                if let Some(version) = decode_at(bytes, base + pad) {
                    return Some(version);
                }
            }
        }
    }

    // Pass 2: fallback for raw standalone VS_FIXEDFILEINFO blocks (e.g. synthetic buffers).
    for offset in 0..=bytes.len().saturating_sub(52) {
        if let Some(version) = decode_at(bytes, offset) {
            return Some(version);
        }
    }
    None
}

/// Extract a dotted numeric browser version (e.g. `131.0.6778.85`) from CLI `--version` text.
pub fn parse_version_string(output: &str) -> Option<String> {
    for token in output.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
        if is_dotted_quad_version(cleaned) {
            return Some(cleaned.to_owned());
        }
    }
    None
}

fn is_dotted_quad_version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    (3..=4).contains(&parts.len())
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

/// Parse Chrome's two-line `DevToolsActivePort` file into `(port, ws_url)`.
pub fn parse_devtools_active_port(content: &str) -> Result<DevToolsEndpoint> {
    let mut lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let port_line = lines.next().ok_or_else(|| {
        DoudouError::new("DEVTOOLS_PORT_INVALID", "DevToolsActivePort file is empty.")
    })?;
    let ws_path = lines.next().ok_or_else(|| {
        DoudouError::new(
            "DEVTOOLS_PORT_INVALID",
            "DevToolsActivePort file is missing the WebSocket path line.",
        )
    })?;
    let port: u16 = port_line.parse().map_err(|err| {
        DoudouError::new(
            "DEVTOOLS_PORT_INVALID",
            format!("Invalid port in DevToolsActivePort: {err}"),
        )
    })?;
    let normalized_path = if ws_path.starts_with('/') {
        ws_path.to_owned()
    } else {
        format!("/{ws_path}")
    };
    Ok(DevToolsEndpoint {
        port,
        ws_url: format!("ws://127.0.0.1:{port}{normalized_path}"),
        browser_version: None,
    })
}

/// Poll `http://127.0.0.1:<port>/json/version` until a valid browser WebSocket URL is returned.
pub async fn poll_json_version(port: u16, timeout: Duration) -> Result<DevToolsEndpoint> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
        .map_err(|err| {
            DoudouError::new(
                "HTTP_CLIENT_ERROR",
                format!("Failed constructing HTTP client for CDP discovery: {err}"),
            )
        })?;
    let url = format!("http://127.0.0.1:{port}/json/version");
    let deadline = Instant::now() + timeout;
    let mut last_error = String::from("no response");

    while Instant::now() < deadline {
        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                if let Ok(payload) = response.json::<serde_json::Value>().await
                    && let Some(ws_url) = payload
                        .get("webSocketDebuggerUrl")
                        .and_then(serde_json::Value::as_str)
                {
                    let browser_version = payload
                        .get("Browser")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    return Ok(DevToolsEndpoint {
                        port,
                        ws_url: ws_url.to_owned(),
                        browser_version,
                    });
                }
                "missing webSocketDebuggerUrl in /json/version".clone_into(&mut last_error);
            }
            Ok(response) => {
                last_error = format!("HTTP {}", response.status());
            }
            Err(err) => {
                last_error = err.to_string();
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err(DoudouError::new(
        "DEVTOOLS_DISCOVERY_TIMEOUT",
        format!("Timed out polling {url} for Chrome WebSocket debugger URL."),
    )
    .with_details(json!({ "port": port, "last_error": last_error })))
}

async fn wait_for_devtools_endpoint(
    child: &mut Child,
    port_file: &Path,
    configured_port: u16,
    timeout: Duration,
) -> Result<DevToolsEndpoint> {
    let deadline = Instant::now() + timeout;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
        .ok();

    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(DoudouError::new(
                "CHROME_LAUNCH_FAILED",
                format!("Chrome exited prematurely before DevTools became ready: {status}"),
            ));
        }

        if let Ok(content) = fs::read_to_string(port_file)
            && let Ok(mut endpoint) = parse_devtools_active_port(&content)
        {
            if let Some(http) = &client
                && let Ok(resp) = http
                    .get(format!("http://127.0.0.1:{}/json/version", endpoint.port))
                    .send()
                    .await
                && let Ok(payload) = resp.json::<serde_json::Value>().await
            {
                if let Some(ws_url) = payload
                    .get("webSocketDebuggerUrl")
                    .and_then(serde_json::Value::as_str)
                {
                    ws_url.clone_into(&mut endpoint.ws_url);
                }
                endpoint.browser_version = payload
                    .get("Browser")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
            }
            return Ok(endpoint);
        }

        if configured_port != 0
            && let Some(http) = &client
            && let Ok(resp) = http
                .get(format!("http://127.0.0.1:{configured_port}/json/version"))
                .send()
                .await
            && resp.status().is_success()
            && let Ok(payload) = resp.json::<serde_json::Value>().await
            && let Some(ws_url) = payload
                .get("webSocketDebuggerUrl")
                .and_then(serde_json::Value::as_str)
        {
            return Ok(DevToolsEndpoint {
                port: configured_port,
                ws_url: ws_url.to_owned(),
                browser_version: payload
                    .get("Browser")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
            });
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err(DoudouError::new(
        "DEVTOOLS_DISCOVERY_TIMEOUT",
        "Chrome started, but neither DevToolsActivePort nor /json/version became ready before timeout.",
    )
    .with_details(json!({
        "port_file": port_file.display().to_string(),
        "configured_port": configured_port,
    })))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn build_launch_args_includes_stealth_flags_and_never_enable_automation() {
        let opts = ChromeLaunchOptions::new(
            PathBuf::from("/tmp/profile/browser-data"),
            LaunchMode::Headless,
        );
        let args = build_launch_args(&opts).expect("stealth flags should build");

        assert!(args.iter().any(|a| a.starts_with("--user-data-dir=")));
        assert!(args.contains(&"--remote-debugging-port=0".to_owned()));
        assert!(args.contains(&"--disable-blink-features=AutomationControlled".to_owned()));
        assert!(args.contains(&"--no-first-run".to_owned()));
        assert!(args.contains(&"--no-default-browser-check".to_owned()));
        assert!(args.contains(&"--disable-infobars".to_owned()));
        assert!(args.contains(&"--lang=zh-CN".to_owned()));
        assert!(args.contains(&"--headless=new".to_owned()));
        assert!(!args.iter().any(|a| a.contains("enable-automation")));
    }

    #[test]
    fn build_launch_args_omits_headless_in_headed_mode() {
        let opts = ChromeLaunchOptions::new(PathBuf::from("/tmp/browser-data"), LaunchMode::Headed);
        let args = build_launch_args(&opts).expect("headed args should build");

        assert!(!args.iter().any(|a| a.starts_with("--headless")));
        assert!(args.contains(&"--disable-blink-features=AutomationControlled".to_owned()));
    }

    #[test]
    fn build_launch_args_rejects_enable_automation_in_extra_args() {
        let opts =
            ChromeLaunchOptions::new(PathBuf::from("/tmp/browser-data"), LaunchMode::Headless)
                .with_extra_arg("--enable-automation");
        let err = build_launch_args(&opts).expect_err("must reject --enable-automation");
        assert_eq!(err.code, "CHROME_STEALTH_VIOLATION");
    }

    #[test]
    fn parse_devtools_active_port_extracts_port_and_ws_url() {
        let raw = "54321\n/devtools/browser/abc-123\n";
        let ep = parse_devtools_active_port(raw).expect("valid DevToolsActivePort");
        assert_eq!(ep.port, 54321);
        assert_eq!(ep.ws_url, "ws://127.0.0.1:54321/devtools/browser/abc-123");
    }

    #[test]
    fn parse_devtools_active_port_rejects_invalid_content() {
        assert!(parse_devtools_active_port("").is_err());
        assert!(parse_devtools_active_port("not-a-port\n/devtools/browser/1").is_err());
        assert!(parse_devtools_active_port("9222\n").is_err());
    }

    #[test]
    fn parse_version_string_handles_chrome_chromium_and_edge() {
        assert_eq!(
            parse_version_string("Google Chrome 131.0.6778.85 \n").as_deref(),
            Some("131.0.6778.85")
        );
        assert_eq!(
            parse_version_string("Chromium 130.0.6723.91 snap").as_deref(),
            Some("130.0.6723.91")
        );
        assert_eq!(
            parse_version_string("Microsoft Edge 129.0.2792.79").as_deref(),
            Some("129.0.2792.79")
        );
        assert_eq!(parse_version_string("no version here"), None);
    }

    #[test]
    fn parse_pe_fixed_file_info_decodes_windows_version_resource() {
        let mut buf = vec![0u8; 64];
        // VS_FIXEDFILEINFO at offset 8
        buf[8..12].copy_from_slice(&0xFEEF_04BDu32.to_le_bytes());
        buf[12..16].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        // 131.0.6778.85 => MS = (131 << 16) | 0, LS = (6778 << 16) | 85
        let ms = 131u32 << 16;
        let ls = (6778u32 << 16) + 85u32;
        buf[16..20].copy_from_slice(&ms.to_le_bytes());
        buf[20..24].copy_from_slice(&ls.to_le_bytes());

        assert_eq!(
            parse_pe_fixed_file_info(&buf).as_deref(),
            Some("131.0.6778.85")
        );
    }

    #[test]
    fn parse_pe_fixed_file_info_prefers_vs_version_info_over_text_decoy() {
        let mut buf = vec![0u8; 256];
        // Decoy signature earlier in the binary (simulating a constant in .text).
        buf[8..12].copy_from_slice(&0xFEEF_04BDu32.to_le_bytes());
        buf[12..16].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        buf[16..20].copy_from_slice(&((9340u32 << 16) + 36168u32).to_le_bytes());
        buf[20..24].copy_from_slice(&((7307u32 << 16) + 19232u32).to_le_bytes());

        // Real .rsrc VS_VERSION_INFO header + 2-byte alignment pad + VS_FIXEDFILEINFO.
        let marker: Vec<u8> = "VS_VERSION_INFO\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        buf[96..96 + marker.len()].copy_from_slice(&marker);
        let real_off = 96 + marker.len() + 2;
        buf[real_off..real_off + 4].copy_from_slice(&0xFEEF_04BDu32.to_le_bytes());
        buf[real_off + 4..real_off + 8].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        buf[real_off + 8..real_off + 12].copy_from_slice(&(158u32 << 16).to_le_bytes());
        buf[real_off + 12..real_off + 16].copy_from_slice(&((8127u32 << 16) + 88u32).to_le_bytes());

        assert_eq!(
            parse_pe_fixed_file_info(&buf).as_deref(),
            Some("158.0.8127.88")
        );
    }

    #[test]
    fn parse_reg_query_default_value_extracts_path() {
        let sample = r"
HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe
    (Default)    REG_SZ    C:\Program Files\Google\Chrome\Application\chrome.exe
";
        assert_eq!(
            parse_reg_query_default_value(sample).as_deref(),
            Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe")
        );
    }

    #[test]
    fn discover_chrome_with_prioritizes_env_override_and_rejects_missing_file() {
        let dir = TempDir::new().expect("tempdir");
        let fake_bin = dir.path().join("chrome.exe");
        fs::write(&fake_bin, b"not-a-real-pe").expect("write stub");

        let found = discover_chrome_with(Some(&fake_bin.to_string_lossy()), &[])
            .expect("explicit binary should resolve");
        assert_eq!(found.path, fake_bin);
        assert_eq!(found.brand, BrowserBrand::Chrome);
        assert_eq!(found.source, DiscoverySource::Explicit);

        let missing = dir.path().join("does-not-exist.exe");
        let err = discover_chrome_with(Some(&missing.to_string_lossy()), &[])
            .expect_err("missing explicit binary must error");
        assert_eq!(err.code, "CHROME_NOT_FOUND");
    }

    #[test]
    fn infer_brand_detects_chrome_chromium_edge_and_custom() {
        assert_eq!(infer_brand(Path::new("chrome.exe")), BrowserBrand::Chrome);
        assert_eq!(
            infer_brand(Path::new("/usr/bin/chromium-browser")),
            BrowserBrand::Chromium
        );
        assert_eq!(infer_brand(Path::new("msedge.exe")), BrowserBrand::Edge);
        assert_eq!(
            infer_brand(Path::new("/opt/custom-browser")),
            BrowserBrand::Custom
        );
    }

    #[tokio::test]
    async fn poll_json_version_discovers_debugger_ws_url_from_http_endpoint() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let expected_ws = format!("ws://127.0.0.1:{port}/devtools/browser/test-id");
        let body = serde_json::json!({
            "Browser": "Chrome/131.0.6778.85",
            "webSocketDebuggerUrl": expected_ws,
        })
        .to_string();

        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut req_buf = [0u8; 512];
                let _ = stream.read(&mut req_buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

        let endpoint = poll_json_version(port, Duration::from_secs(3))
            .await
            .expect("poll_json_version");
        assert_eq!(endpoint.port, port);
        assert_eq!(endpoint.ws_url, expected_ws);
        assert_eq!(
            endpoint.browser_version.as_deref(),
            Some("Chrome/131.0.6778.85")
        );
    }
}
