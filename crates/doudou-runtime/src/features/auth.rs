//! Headless QR code and SMS authentication flows kept alive inside the profile broker.
//!
//! Supports two complementary workflows without launching a headed browser window:
//! - **Agent / non-blocking**: `session.auth.start` opens Douyin in headless Chrome, extracts the
//!   QR PNG to disk, and returns immediately while keeping the page alive; callers can inspect
//!   `session.auth.status`, call `session.auth.refresh_qr`, complete SMS with
//!   `session.auth.sms_send` / `session.auth.sms_verify`, or wait with `session.auth.wait`.
//! - **Terminal / blocking**: `doudou session login --headless` runs start, prints the QR image
//!   path, and waits until authentication succeeds or secondary verification is required.
//!
//! Never attempts to solve or bypass CAPTCHA/slider challenges: if a slider or passkey challenge
//! appears, every operation fails closed with `VERIFICATION_REQUIRED`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use doudou_cdp::{
    CdpPageOps, ChromeSession, EvalWorld, KeyEventSpec, KeyModifiers, LoginClassification,
    MouseButton, NetworkCaptureBuffer, ScreenshotClip, SessionMode, VerificationKind,
};
use doudou_core::{DoudouError, Result, atomic_write_bytes, profile_paths, sha256_bytes, utc_now};
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

const PASSPORT_CAPTURE_PATTERNS: &[&str] = &[
    "/passport/web/get_qrcode/",
    "/passport/web/check_qrconnect/",
    "/passport/web/send_code/",
    "/passport/web/sms_login/",
];

const PANEL_PROBE_JS: &str = r#"(() => {
  const isVisible = (el) => {
    if (!el) return false;
    const style = window.getComputedStyle(el);
    if (!style || style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 1 && rect.height > 1;
  };
  const rectJson = (el) => {
    if (!el) return null;
    const r = el.getBoundingClientRect();
    if (r.width <= 1 || r.height <= 1) return null;
    return { x: r.x, y: r.y, width: r.width, height: r.height };
  };
  const firstVisible = (sel) => Array.from(document.querySelectorAll(sel)).find(isVisible) || null;

  const qrSelectors = [
    '#douyin_login_comp_scan_code_qrcode img',
    '[id*="qrcode" i] img',
    '[class*="qrcode" i] img',
    '[class*="qr-code" i] img',
    '[class*="login" i] img[src^="data:image/"]',
    '[class*="login" i] canvas[class*="qr" i]'
  ];
  const qrEl = firstVisible(qrSelectors.join(', '));
  let qrDataUrl = null;
  let qrMatrix = null;
  if (qrEl) {
    if (qrEl.tagName === 'IMG' && typeof qrEl.src === 'string' && qrEl.src.startsWith('data:image/')) {
      qrDataUrl = qrEl.src;
    } else if (qrEl.tagName === 'CANVAS') {
      try { qrDataUrl = qrEl.toDataURL('image/png'); } catch {}
    }
    try {
      const w = qrEl.tagName === 'IMG' ? qrEl.naturalWidth : qrEl.width;
      const h = qrEl.tagName === 'IMG' ? qrEl.naturalHeight : qrEl.height;
      const ready = qrEl.tagName === 'IMG' ? (qrEl.complete && w >= 21 && h >= 21) : (w >= 21 && h >= 21);
      if (ready && w <= 2048 && h <= 2048) {
        const canvas = document.createElement('canvas');
        canvas.width = w;
        canvas.height = h;
        const ctx = canvas.getContext('2d', { willReadFrequently: true });
        if (ctx) {
          ctx.fillStyle = '#ffffff';
          ctx.fillRect(0, 0, w, h);
          ctx.drawImage(qrEl, 0, 0, w, h);
          const pixels = ctx.getImageData(0, 0, w, h).data;
          const isDark = (x, y) => {
            const idx = (y * w + x) * 4;
            return (pixels[idx] + pixels[idx + 1] + pixels[idx + 2]) < 384;
          };
          let minX = w, maxX = -1, minY = h, maxY = -1;
          for (let y = 0; y < h; y++) {
            for (let x = 0; x < w; x++) {
              if (isDark(x, y)) {
                if (x < minX) minX = x;
                if (x > maxX) maxX = x;
                if (y < minY) minY = y;
                if (y > maxY) maxY = y;
              }
            }
          }
          const bw = maxX - minX + 1;
          const bh = maxY - minY + 1;
          if (bw >= 21 && bh >= 21 && Math.abs(bw - bh) <= Math.max(4, bw * 0.05)) {
            let finderRun = 0;
            const probeY = Math.min(maxY, minY + Math.max(0, Math.floor(bh / 200)));
            for (let x = minX; x <= maxX; x++) {
              if (isDark(x, probeY)) finderRun++;
              else if (finderRun > 0) break;
            }
            if (finderRun >= 7) {
              const rawGrid = bw / (finderRun / 7);
              const k = Math.max(0, Math.round((rawGrid - 21) / 4));
              const grid = 21 + 4 * k;
              if (grid >= 21 && grid <= 81 && Math.abs(grid - rawGrid) <= 1.5) {
                const cellW = bw / grid;
                const cellH = bh / grid;
                const rows = [];
                for (let r = 0; r < grid; r++) {
                  const sy = Math.min(maxY, Math.floor(minY + (r + 0.5) * cellH));
                  let row = '';
                  for (let c = 0; c < grid; c++) {
                    const sx = Math.min(maxX, Math.floor(minX + (c + 0.5) * cellW));
                    row += isDark(sx, sy) ? '1' : '0';
                  }
                  rows.push(row);
                }
                qrMatrix = rows;
              }
            }
          }
        }
      }
    } catch {}
  }
  const qrContainer = firstVisible('#douyin_login_comp_scan_code_qrcode, #douyin_login_comp_scan_code, #douyin_login_comp_scan_code_login_content, [id*="qrcode" i], [class*="qrcode" i]');
  const hasQrPanel = Boolean(qrContainer);
  const qrContainerText = qrContainer ? (qrContainer.innerText || '').trim() : '';
  const qrExpired = /失效|过期|超时/.test(qrContainerText);
  let refreshRect = null;
  if (qrContainer && qrExpired) {
    const refreshCandidate = Array.from(qrContainer.querySelectorAll('button, [role="button"], span, div, svg'))
      .find((el) => isVisible(el) && /刷新|重新/.test((el.innerText || '').trim()));
    refreshRect = rectJson(refreshCandidate || qrContainer);
  }

  const panel = firstVisible('#douyin_login_comp_flat_panel, #login-panel-new, #douyin-login-new-id, .second-verify-mask, [role="dialog"]');
  const phoneEl = firstVisible('input[name="normal-input"][placeholder*="手机号"], input[placeholder*="手机号"]');
  const codeEl = firstVisible('input[name="button-input"][placeholder*="验证码"], .second-verify-mask input, input[placeholder*="验证码"], input[maxlength="6"]');
  const smsTabEl = firstVisible('#douyin_login_comp_mobile_title');

  const panelScope = panel || document.body;
  const getCodeEl = Array.from(panelScope.querySelectorAll('button, [role="button"], span, div'))
    .filter((el) => isVisible(el) && el.children.length === 0)
    .find((el) => /^(获取验证码|发送验证码|重新发送|重新获取)$/.test((el.innerText || '').trim()));
  const countdownEl = Array.from(panelScope.querySelectorAll('button, [role="button"], span, div'))
    .filter((el) => isVisible(el) && el.children.length === 0)
    .find((el) => /^\d+\s*[s秒]/.test((el.innerText || '').trim()) || /后重发|重新发送\(\d+/.test((el.innerText || '').trim()));

  const submitEl = firstVisible('#douyin_login_comp_btn_id') ||
    Array.from(panelScope.querySelectorAll('button, [role="button"], div'))
      .filter((el) => isVisible(el) && el.children.length === 0)
      .find((el) => /^(登录|确定|验证|确认|下一步)$/.test((el.innerText || '').trim()));

  let inlineError = null;
  if (panel) {
    const errEl = Array.from(panel.querySelectorAll('[class*="error" i], [class*="tip" i], [role="alert"], p, span, div'))
      .filter((el) => isVisible(el) && el.children.length === 0)
      .map((el) => (el.innerText || '').trim())
      .find((t) => t.length >= 3 && t.length <= 80 && /(错误|无效|不正确|频繁|上限|格式|失败|请先|请输入正确)/.test(t));
    inlineError = errEl || null;
  }

  return {
    has_qr_panel: hasQrPanel,
    qr_data_url: qrDataUrl,
    qr_matrix: qrMatrix,
    qr_rect: rectJson(qrEl),
    qr_expired: qrExpired,
    refresh_rect: refreshRect,
    sms_available: Boolean(codeEl || phoneEl || smsTabEl),
    phone_rect: rectJson(phoneEl),
    code_rect: rectJson(codeEl),
    sms_tab_rect: rectJson(smsTabEl),
    get_code_rect: rectJson(getCodeEl),
    get_code_text: getCodeEl ? (getCodeEl.innerText || '').trim() : null,
    sms_countdown_active: Boolean(countdownEl),
    submit_rect: rectJson(submitEl),
    inline_error: inlineError,
  };
})()"#;

#[derive(Debug, Clone, Default)]
struct PanelSnapshot {
    has_qr_panel: bool,
    qr_data_url: Option<String>,
    qr_matrix: Option<Vec<String>>,
    qr_rect: Option<Rect>,
    qr_expired: bool,
    refresh_rect: Option<Rect>,
    sms_available: bool,
    phone_rect: Option<Rect>,
    code_rect: Option<Rect>,
    sms_tab_rect: Option<Rect>,
    get_code_rect: Option<Rect>,
    sms_countdown_active: bool,
    submit_rect: Option<Rect>,
    inline_error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Rect {
    fn from_value(v: Option<&Value>) -> Option<Self> {
        let obj = v?.as_object()?;
        let x = obj.get("x")?.as_f64()?;
        let y = obj.get("y")?.as_f64()?;
        let width = obj.get("width")?.as_f64()?;
        let height = obj.get("height")?.as_f64()?;
        if width > 1.0 && height > 1.0 && x.is_finite() && y.is_finite() {
            Some(Self {
                x,
                y,
                width,
                height,
            })
        } else {
            None
        }
    }

    fn center(self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

/// Mask a phone number so raw digits never appear in JSON responses or logs.
pub fn mask_phone(phone: &str) -> String {
    let digits: Vec<char> = phone.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() <= 4 {
        return "****".to_owned();
    }
    if digits.len() <= 7 {
        let tail: String = digits[digits.len() - 2..].iter().collect();
        return format!("****{tail}");
    }
    let head: String = digits[..3].iter().collect();
    let tail: String = digits[digits.len() - 4..].iter().collect();
    format!("{head}****{tail}")
}

/// Decode a `data:image/...;base64,<payload>` URL into raw bytes.
pub fn decode_data_image_url(data_url: &str) -> Option<Vec<u8>> {
    let comma = data_url.find(',')?;
    let header = &data_url[..comma];
    if !header.starts_with("data:image/") || !header.contains(";base64") {
        return None;
    }
    let b64 = data_url[comma + 1..].trim();
    BASE64.decode(b64).ok().filter(|bytes| !bytes.is_empty())
}

/// Rendered terminal-scannable QR code derived from a square binary module matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalQr {
    /// Grid dimension (`N` for an `N x N` QR symbol, excluding quiet zone).
    pub modules: usize,
    /// ANSI black-on-white (`\x1b[30;47m`) Unicode half-block QR safe for dark and light terminals.
    pub ansi: String,
    /// Plain ANSI-free Unicode half-block QR (inverted for dark-background terminals/code blocks).
    pub text: String,
}

/// Render a square `"0"`/`"1"` QR module matrix into compact Unicode half-block terminal art.
pub fn render_qr_terminal(matrix: &[String]) -> Option<TerminalQr> {
    const QUIET: usize = 2;
    let size = matrix.len();
    if !(21..=81).contains(&size) {
        return None;
    }
    let mut grid = Vec::with_capacity(size);
    for row in matrix {
        if row.len() != size {
            return None;
        }
        let mut bits = Vec::with_capacity(size);
        for b in row.bytes() {
            match b {
                b'1' => bits.push(true),
                b'0' => bits.push(false),
                _ => return None,
            }
        }
        grid.push(bits);
    }

    let total = size + QUIET * 2;
    let is_dark = |r: usize, c: usize| -> bool {
        if r < QUIET || c < QUIET || r >= size + QUIET || c >= size + QUIET {
            false
        } else {
            grid[r - QUIET][c - QUIET]
        }
    };

    let mut ansi_lines = Vec::with_capacity(total.div_ceil(2));
    let mut text_lines = Vec::with_capacity(total.div_ceil(2));

    let mut r = 0;
    while r < total {
        let mut ansi_line = String::from("\x1b[30;47m");
        let mut text_line = String::with_capacity(total);
        for c in 0..total {
            let top_dark = is_dark(r, c);
            let bot_dark = if r + 1 < total {
                is_dark(r + 1, c)
            } else {
                false
            };
            // In ANSI black-fg (30) on white-bg (47), filled block = dark module, space = light.
            let ansi_ch = match (top_dark, bot_dark) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            };
            // In plain text (typically rendered in dark terminals/code blocks), light = filled block.
            let text_ch = match (!top_dark, !bot_dark) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            };
            ansi_line.push(ansi_ch);
            text_line.push(text_ch);
        }
        ansi_line.push_str("\x1b[0m");
        ansi_lines.push(ansi_line);
        text_lines.push(text_line);
        r += 2;
    }

    Some(TerminalQr {
        modules: size,
        ansi: ansi_lines.join("\n"),
        text: text_lines.join("\n"),
    })
}

fn parse_panel_snapshot(val: &Value) -> PanelSnapshot {
    let qr_matrix = val
        .get("qr_matrix")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|rows| !rows.is_empty());
    PanelSnapshot {
        has_qr_panel: val
            .get("has_qr_panel")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        qr_data_url: val
            .get("qr_data_url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        qr_matrix,
        qr_rect: Rect::from_value(val.get("qr_rect")),
        qr_expired: val
            .get("qr_expired")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        refresh_rect: Rect::from_value(val.get("refresh_rect")),
        sms_available: val
            .get("sms_available")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        phone_rect: Rect::from_value(val.get("phone_rect")),
        code_rect: Rect::from_value(val.get("code_rect")),
        sms_tab_rect: Rect::from_value(val.get("sms_tab_rect")),
        get_code_rect: Rect::from_value(val.get("get_code_rect")),
        sms_countdown_active: val
            .get("sms_countdown_active")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        submit_rect: Rect::from_value(val.get("submit_rect")),
        inline_error: val
            .get("inline_error")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    }
}

async fn probe_panel(page: &dyn CdpPageOps) -> PanelSnapshot {
    match page.eval_json(PANEL_PROBE_JS, EvalWorld::Isolated).await {
        Ok(val) => parse_panel_snapshot(&val),
        Err(_) => PanelSnapshot::default(),
    }
}

#[derive(Debug, Default)]
struct PassportTelemetry {
    qrcode_png: Option<Vec<u8>>,
    expire_time: Option<i64>,
    qr_status: Option<String>,
    send_code_error: Option<String>,
    send_code_ok: bool,
    sms_login_error: Option<String>,
}

fn inspect_passport_capture(buffer: &NetworkCaptureBuffer) -> PassportTelemetry {
    let mut out = PassportTelemetry::default();
    for ex in buffer.snapshot() {
        let Some(body) = ex.response_body.as_deref() else {
            continue;
        };
        let Ok(json) = serde_json::from_slice::<Value>(body) else {
            continue;
        };
        let data = json.get("data").and_then(Value::as_object);
        if ex.url.contains("/passport/web/get_qrcode/") {
            if let Some(d) = data {
                if let Some(b64) = d.get("qrcode").and_then(Value::as_str)
                    && let Ok(bytes) = BASE64.decode(b64.trim())
                    && !bytes.is_empty()
                {
                    out.qrcode_png = Some(bytes);
                }
                if let Some(exp) = d.get("expire_time").and_then(Value::as_i64) {
                    out.expire_time = Some(exp);
                }
            }
        } else if ex.url.contains("/passport/web/check_qrconnect/") {
            if let Some(st) = data
                .and_then(|d| d.get("status"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                out.qr_status = Some(st.to_owned());
            }
        } else if ex.url.contains("/passport/web/send_code/") {
            let err_code = data
                .and_then(|d| d.get("error_code"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let msg = json
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if err_code == 0 && (msg.is_empty() || msg == "success") {
                out.send_code_ok = true;
                out.send_code_error = None;
            } else {
                let desc = data
                    .and_then(|d| d.get("description"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Douyin rejected the SMS code request.");
                out.send_code_error = Some(desc.to_owned());
            }
        } else if ex.url.contains("/passport/web/sms_login/") {
            let err_code = data
                .and_then(|d| d.get("error_code"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            if err_code != 0 {
                let desc = data
                    .and_then(|d| d.get("description"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Douyin rejected the SMS verification code.");
                out.sms_login_error = Some(desc.to_owned());
            } else {
                out.sms_login_error = None;
            }
        }
    }
    out
}

fn persist_qr_png(
    home: &Path,
    data_root: &Path,
    profile_name: &str,
    png_bytes: &[u8],
) -> Result<(PathBuf, PathBuf, String)> {
    let paths = profile_paths(home, data_root, profile_name)?;
    let auth_dir = paths.diagnostics.join("auth");
    fs::create_dir_all(&auth_dir).map_err(|err| {
        DoudouError::new(
            "IO_ERROR",
            format!("Failed creating auth diagnostics directory: {err}"),
        )
    })?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let sha = sha256_bytes(png_bytes);
    let short = &sha[..8];
    let versioned = auth_dir.join(format!("qr-{stamp}-{short}.png"));
    let latest = auth_dir.join("latest-qr.png");
    atomic_write_bytes(&versioned, png_bytes, None)?;
    atomic_write_bytes(&latest, png_bytes, None)?;
    Ok((versioned, latest, sha))
}

async fn extract_qr_png(
    page: &dyn CdpPageOps,
    panel: &PanelSnapshot,
    telemetry: &PassportTelemetry,
) -> Option<(Vec<u8>, &'static str)> {
    if let Some(ref data_url) = panel.qr_data_url
        && let Some(bytes) = decode_data_image_url(data_url)
    {
        return Some((bytes, "dom_data_url"));
    }
    if let Some(ref bytes) = telemetry.qrcode_png {
        return Some((bytes.clone(), "passport_api"));
    }
    if let Some(rect) = panel.qr_rect
        && rect.width >= 60.0
        && rect.height >= 60.0
        && let Ok(bytes) = page
            .screenshot_clip_png(ScreenshotClip {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                scale: 2.0,
            })
            .await
        && !bytes.is_empty()
    {
        return Some((bytes, "cdp_screenshot"));
    }
    None
}

async fn enable_passport_capture(page: &dyn CdpPageOps) {
    let patterns = PASSPORT_CAPTURE_PATTERNS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let _ = page.enable_network_capture(patterns).await;
}

async fn check_and_settle_authenticated(
    guard: &mut ChromeSession,
    settle_budget: Duration,
) -> Result<Option<LoginClassification>> {
    let initial = guard.classify_login().await?;
    if initial.valid {
        return Ok(Some(initial));
    }
    if guard.mode() == SessionMode::Fake {
        return Ok(None);
    }
    if !initial.verification_visible && guard.has_douyin_auth_cookie().await {
        let current_url = match guard.ensure_page().await {
            Ok(p) => p.current_url().await.unwrap_or_default(),
            Err(_) => String::new(),
        };
        if !current_url.starts_with("https://www.douyin.com/chat") {
            let _ = guard.navigate_douyin("/chat").await;
        }
        let deadline = Instant::now() + settle_budget;
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let probe = guard.classify_login().await?;
            if probe.valid {
                return Ok(Some(probe));
            }
            if probe.verification_visible {
                return Ok(None);
            }
        }
    }
    Ok(None)
}

async fn focus_and_replace_input(page: &dyn CdpPageOps, rect: Rect, text: &str) -> Result<()> {
    let (cx, cy) = rect.center();
    page.click_xy(cx, cy, MouseButton::Left, 1).await?;
    tokio::time::sleep(Duration::from_millis(80)).await;
    page.dispatch_key(&KeyEventSpec {
        key: "a".to_owned(),
        code: Some("KeyA".to_owned()),
        windows_virtual_key_code: Some(65),
        modifiers: KeyModifiers {
            ctrl: !cfg!(target_os = "macos"),
            meta: cfg!(target_os = "macos"),
            ..KeyModifiers::default()
        },
        text: None,
    })
    .await?;
    page.dispatch_key(&KeyEventSpec {
        key: "Backspace".to_owned(),
        code: Some("Backspace".to_owned()),
        windows_virtual_key_code: Some(8),
        modifiers: KeyModifiers::default(),
        text: None,
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    page.insert_text(text).await?;
    Ok(())
}

fn verification_kind_str(kind: Option<VerificationKind>) -> Option<&'static str> {
    match kind {
        Some(VerificationKind::SmsCode) => Some("sms_code"),
        Some(VerificationKind::QuickLoginSms) => Some("quick_login_sms"),
        Some(VerificationKind::Slider) => Some("slider"),
        Some(VerificationKind::QrLogin) => Some("qr_login"),
        Some(VerificationKind::Passkey) => Some("passkey"),
        Some(VerificationKind::GenericVerification | _) => Some("generic_verification"),
        None => None,
    }
}

fn is_blocking_captcha(kind: Option<VerificationKind>) -> bool {
    matches!(
        kind,
        Some(
            VerificationKind::Slider
                | VerificationKind::Passkey
                | VerificationKind::GenericVerification
        )
    )
}

/// Start headless authentication: open Douyin in headless Chrome, capture QR PNG to disk,
/// and return immediately while keeping the page alive inside the broker.
pub async fn auth_start(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile_name: &str,
    params: &Value,
) -> Result<Value> {
    let force_reload = params
        .get("force_reload")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut guard = session.lock().await;
    let is_fake = guard.mode() == SessionMode::Fake;
    let page = guard.ensure_page().await?;
    enable_passport_capture(page.as_ref()).await;

    let current_url = page.current_url().await.unwrap_or_default();
    if force_reload || !current_url.starts_with("https://www.douyin.com") {
        page.capture_buffer().clear();
        let _ = guard.navigate_douyin("/chat").await?;
    }

    if let Some(login) = check_and_settle_authenticated(&mut guard, Duration::from_secs(6)).await? {
        let status = guard.status().await;
        return Ok(json!({
            "status": "authenticated",
            "profile_id": profile_name,
            "login": login,
            "page_url": status.page_url,
            "checked_at": utc_now(),
        }));
    }

    let page = guard.ensure_page().await?;
    let capture_buf = page.capture_buffer();
    let wait_budget = if is_fake {
        Duration::from_millis(200)
    } else {
        Duration::from_secs(15)
    };
    let deadline = Instant::now() + wait_budget;

    loop {
        let login = guard.classify_login().await?;
        if login.valid {
            let status = guard.status().await;
            return Ok(json!({
                "status": "authenticated",
                "profile_id": profile_name,
                "login": login,
                "page_url": status.page_url,
                "checked_at": utc_now(),
            }));
        }
        if is_blocking_captcha(login.verification_kind) {
            return Ok(json!({
                "status": "verification_required",
                "profile_id": profile_name,
                "verification_kind": verification_kind_str(login.verification_kind),
                "message": "A slider or passkey challenge is blocking headless login. Run `doudou session login` in a local desktop terminal to complete it interactively.",
                "checked_at": utc_now(),
            }));
        }

        let panel = probe_panel(page.as_ref()).await;
        let telemetry = inspect_passport_capture(&capture_buf);

        if let Some((png_bytes, source)) = extract_qr_png(page.as_ref(), &panel, &telemetry).await {
            let (versioned, latest, sha) =
                persist_qr_png(home, data_root, profile_name, &png_bytes)?;
            let page_url = page.current_url().await.unwrap_or_default();
            let terminal_qr = panel.qr_matrix.as_deref().and_then(render_qr_terminal);
            let payload = json!({
                "status": if panel.qr_expired { "qr_expired" } else { "qr_ready" },
                "profile_id": profile_name,
                "qr_image_path": versioned.display().to_string(),
                "qr_latest_path": latest.display().to_string(),
                "qr_sha256": sha,
                "qr_source": source,
                "qr_status": telemetry.qr_status.unwrap_or_else(|| "new".to_owned()),
                "qr_modules": terminal_qr.as_ref().map(|q| q.modules),
                "qr_terminal": terminal_qr.as_ref().map(|q| q.ansi.as_str()),
                "qr_text": terminal_qr.as_ref().map(|q| q.text.as_str()),
                "expire_time": telemetry.expire_time,
                "sms_available": panel.sms_available,
                "verification_kind": verification_kind_str(login.verification_kind),
                "page_url": page_url,
                "checked_at": utc_now(),
            });
            if let Some(auth_dir) = latest.parent()
                && let Ok(meta_bytes) = serde_json::to_vec_pretty(&payload)
            {
                let _ = atomic_write_bytes(&auth_dir.join("latest-qr.json"), &meta_bytes, None);
            }
            return Ok(payload);
        }

        if !panel.has_qr_panel
            && matches!(
                login.verification_kind,
                Some(VerificationKind::SmsCode | VerificationKind::QuickLoginSms)
            )
        {
            let page_url = page.current_url().await.unwrap_or_default();
            return Ok(json!({
                "status": "sms_required",
                "profile_id": profile_name,
                "sms_available": true,
                "verification_kind": verification_kind_str(login.verification_kind),
                "page_url": page_url,
                "checked_at": utc_now(),
            }));
        }

        if Instant::now() >= deadline {
            return Err(DoudouError::new(
                "AUTH_QR_NOT_READY",
                "Timed out waiting for the Douyin QR login modal or SMS form in headless Chrome.",
            )
            .with_details(json!({
                "profile_id": profile_name,
                "verification_kind": verification_kind_str(login.verification_kind),
                "sms_available": panel.sms_available,
            })));
        }
        tokio::time::sleep(Duration::from_millis(if is_fake { 20 } else { 300 })).await;
    }
}

/// Inspect the live headless authentication page without reloading it.
pub async fn auth_status(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile_name: &str,
) -> Result<Value> {
    let mut guard = session.lock().await;
    let paths = profile_paths(home, data_root, profile_name)?;
    let latest_qr = paths.diagnostics.join("auth").join("latest-qr.png");
    let latest_qr_opt = latest_qr.is_file().then(|| latest_qr.display().to_string());

    if let Some(login) = check_and_settle_authenticated(&mut guard, Duration::from_secs(5)).await? {
        let status = guard.status().await;
        return Ok(json!({
            "status": "authenticated",
            "profile_id": profile_name,
            "login": login,
            "page_url": status.page_url,
            "checked_at": utc_now(),
        }));
    }

    let page = guard.ensure_page().await?;
    let capture_buf = page.capture_buffer();
    let login = guard.classify_login().await?;
    let panel = probe_panel(page.as_ref()).await;
    let telemetry = inspect_passport_capture(&capture_buf);
    let page_url = page.current_url().await.unwrap_or_default();

    let state = if is_blocking_captcha(login.verification_kind) {
        "verification_required"
    } else if matches!(
        login.verification_kind,
        Some(VerificationKind::SmsCode | VerificationKind::QuickLoginSms)
    ) {
        "sms_required"
    } else if panel.qr_expired || telemetry.qr_status.as_deref() == Some("expired") {
        "qr_expired"
    } else if telemetry.qr_status.as_deref() == Some("scanned") {
        "qr_scanned"
    } else if panel.qr_data_url.is_some() || panel.qr_rect.is_some() {
        "qr_ready"
    } else {
        "login_required"
    };

    Ok(json!({
        "status": state,
        "profile_id": profile_name,
        "qr_latest_path": latest_qr_opt,
        "qr_status": telemetry.qr_status,
        "expire_time": telemetry.expire_time,
        "sms_available": panel.sms_available,
        "sms_countdown_active": panel.sms_countdown_active,
        "inline_error": panel.inline_error,
        "verification_kind": verification_kind_str(login.verification_kind),
        "page_url": page_url,
        "checked_at": utc_now(),
    }))
}

/// Refresh an expired or stale QR code on the live headless page and save the updated PNG.
pub async fn auth_refresh_qr(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile_name: &str,
) -> Result<Value> {
    let previous_sha = {
        let paths = profile_paths(home, data_root, profile_name)?;
        let latest = paths.diagnostics.join("auth").join("latest-qr.png");
        fs::read(&latest).ok().map(|b| sha256_bytes(&b))
    };

    {
        let mut guard = session.lock().await;
        let page = guard.ensure_page().await?;
        page.capture_buffer().clear();
        enable_passport_capture(page.as_ref()).await;
        let panel = probe_panel(page.as_ref()).await;
        if let Some(refresh_rect) = panel.refresh_rect {
            let (cx, cy) = refresh_rect.center();
            let _ = page.click_xy(cx, cy, MouseButton::Left, 1).await;
            tokio::time::sleep(Duration::from_millis(
                if guard.mode() == SessionMode::Fake {
                    20
                } else {
                    800
                },
            ))
            .await;
        } else {
            let _ = guard.navigate_douyin("/chat").await?;
        }
    }

    let mut result = auth_start(session, home, data_root, profile_name, &json!({})).await?;
    if let Some(obj) = result.as_object_mut() {
        let new_sha = obj
            .get("qr_sha256")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        obj.insert(
            "qr_changed".to_owned(),
            json!(previous_sha.is_some() && previous_sha != new_sha),
        );
    }
    Ok(result)
}

/// Enter optional phone number and trigger Douyin's "获取验证码" / "发送验证码" control.
pub async fn auth_sms_send(
    session: &Arc<AsyncMutex<ChromeSession>>,
    _home: &Path,
    _data_root: &Path,
    profile_name: &str,
    params: &Value,
) -> Result<Value> {
    let phone = params
        .get("phone")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let mut guard = session.lock().await;
    let is_fake = guard.mode() == SessionMode::Fake;
    let page = guard.ensure_page().await?;
    enable_passport_capture(page.as_ref()).await;

    let current_url = page.current_url().await.unwrap_or_default();
    if !current_url.starts_with("https://www.douyin.com") {
        page.capture_buffer().clear();
        let _ = guard.navigate_douyin("/chat").await?;
        if !is_fake {
            tokio::time::sleep(Duration::from_millis(2000)).await;
        }
    }

    let pre_login = guard.classify_login().await?;
    if pre_login.valid {
        let status = guard.status().await;
        return Ok(json!({
            "status": "authenticated",
            "profile_id": profile_name,
            "login": pre_login,
            "page_url": status.page_url,
            "checked_at": utc_now(),
        }));
    }
    if is_blocking_captcha(pre_login.verification_kind) {
        return Ok(json!({
            "status": "verification_required",
            "profile_id": profile_name,
            "verification_kind": verification_kind_str(pre_login.verification_kind),
            "message": "A slider or passkey challenge is blocking SMS login. Run `doudou session login` in a local desktop terminal to complete it interactively.",
            "checked_at": utc_now(),
        }));
    }

    let page = guard.ensure_page().await?;
    let mut panel = probe_panel(page.as_ref()).await;

    if panel.phone_rect.is_none()
        && panel.code_rect.is_none()
        && let Some(tab_rect) = panel.sms_tab_rect
    {
        let (cx, cy) = tab_rect.center();
        page.click_xy(cx, cy, MouseButton::Left, 1).await?;
        tokio::time::sleep(Duration::from_millis(if is_fake { 20 } else { 400 })).await;
        panel = probe_panel(page.as_ref()).await;
    }

    if let Some(ref raw_phone) = phone {
        let Some(phone_rect) = panel.phone_rect else {
            return Err(DoudouError::new(
                "AUTH_SMS_FORM_NOT_FOUND",
                "Could not locate a visible phone number input on the Douyin login panel.",
            ));
        };
        focus_and_replace_input(page.as_ref(), phone_rect, raw_phone).await?;
        tokio::time::sleep(Duration::from_millis(if is_fake { 20 } else { 200 })).await;
        let mid_login = guard.classify_login().await?;
        if is_blocking_captcha(mid_login.verification_kind) {
            return Ok(json!({
                "status": "verification_required",
                "profile_id": profile_name,
                "verification_kind": verification_kind_str(mid_login.verification_kind),
                "message": "Douyin presented a slider or passkey challenge before sending the SMS code. Complete it via `doudou session login`.",
                "checked_at": utc_now(),
            }));
        }
        panel = probe_panel(page.as_ref()).await;
    }

    let Some(get_code_rect) = panel.get_code_rect else {
        let cooldown_login = guard.classify_login().await?;
        if is_blocking_captcha(cooldown_login.verification_kind) {
            return Ok(json!({
                "status": "verification_required",
                "profile_id": profile_name,
                "verification_kind": verification_kind_str(cooldown_login.verification_kind),
                "message": "A slider or passkey challenge is visible on the login panel. Complete it via `doudou session login`.",
                "checked_at": utc_now(),
            }));
        }
        if panel.sms_countdown_active {
            return Ok(json!({
                "status": "sms_sent",
                "profile_id": profile_name,
                "phone_masked": phone.as_deref().map(mask_phone),
                "already_cooling_down": true,
                "checked_at": utc_now(),
            }));
        }
        return Err(DoudouError::new(
            "AUTH_SMS_BUTTON_NOT_FOUND",
            "Could not locate a visible '获取验证码' or '发送验证码' button on the page.",
        ));
    };

    let pre_click_error = panel.inline_error.clone();
    page.capture_buffer().clear();
    let (cx, cy) = get_code_rect.center();
    page.click_xy(cx, cy, MouseButton::Left, 1).await?;

    let submitted_at = Instant::now();
    let dom_error_grace = if is_fake {
        Duration::from_millis(30)
    } else {
        Duration::from_millis(1200)
    };
    let settle = if is_fake {
        Duration::from_millis(60)
    } else {
        Duration::from_secs(3)
    };
    let deadline = submitted_at + settle;
    let capture_buf = page.capture_buffer();

    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(if is_fake { 20 } else { 250 })).await;
        let login = guard.classify_login().await?;
        if is_blocking_captcha(login.verification_kind) {
            return Ok(json!({
                "status": "verification_required",
                "profile_id": profile_name,
                "verification_kind": verification_kind_str(login.verification_kind),
                "message": "Douyin presented a slider or security challenge after requesting an SMS code. Use `doudou session login` in a local desktop terminal to complete it.",
                "checked_at": utc_now(),
            }));
        }
        let after = probe_panel(page.as_ref()).await;
        let telemetry = inspect_passport_capture(&capture_buf);
        let fresh_dom_error = after.inline_error.clone().filter(|msg| {
            pre_click_error.as_deref() != Some(msg.as_str())
                || submitted_at.elapsed() >= dom_error_grace
        });
        if let Some(err_msg) = telemetry.send_code_error.or(fresh_dom_error) {
            return Err(
                DoudouError::new("AUTH_SMS_SEND_FAILED", err_msg).with_details(json!({
                    "profile_id": profile_name,
                    "phone_masked": phone.as_deref().map(mask_phone),
                })),
            );
        }
        if after.sms_countdown_active || telemetry.send_code_ok {
            return Ok(json!({
                "status": "sms_sent",
                "profile_id": profile_name,
                "phone_masked": phone.as_deref().map(mask_phone),
                "countdown_active": after.sms_countdown_active,
                "checked_at": utc_now(),
            }));
        }
    }

    Err(DoudouError::new(
        "AUTH_SMS_SEND_FAILED",
        "Timed out waiting for Douyin to confirm the SMS code request.",
    )
    .with_details(json!({
        "profile_id": profile_name,
        "phone_masked": phone.as_deref().map(mask_phone),
        "countdown_active": false,
    })))
}

/// Fill the user-supplied SMS verification code and submit the Douyin login/verify form.
pub async fn auth_sms_verify(
    session: &Arc<AsyncMutex<ChromeSession>>,
    _home: &Path,
    _data_root: &Path,
    profile_name: &str,
    params: &Value,
) -> Result<Value> {
    let code = params
        .get("code")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            DoudouError::new(
                "MISSING_ARGUMENT",
                "A non-empty SMS verification code is required.",
            )
        })?;

    let mut guard = session.lock().await;
    let is_fake = guard.mode() == SessionMode::Fake;
    let page = guard.ensure_page().await?;
    enable_passport_capture(page.as_ref()).await;

    let pre_login = guard.classify_login().await?;
    if pre_login.valid {
        let status = guard.status().await;
        return Ok(json!({
            "status": "authenticated",
            "profile_id": profile_name,
            "login": pre_login,
            "page_url": status.page_url,
            "checked_at": utc_now(),
        }));
    }
    if is_blocking_captcha(pre_login.verification_kind) {
        return Ok(json!({
            "status": "verification_required",
            "profile_id": profile_name,
            "verification_kind": verification_kind_str(pre_login.verification_kind),
            "message": "A slider or passkey challenge is blocking SMS verification. Complete it via `doudou session login`.",
            "checked_at": utc_now(),
        }));
    }

    let panel = probe_panel(page.as_ref()).await;
    let Some(code_rect) = panel.code_rect else {
        return Err(DoudouError::new(
            "AUTH_SMS_FORM_NOT_FOUND",
            "Could not locate a visible SMS verification code input on the page.",
        ));
    };

    focus_and_replace_input(page.as_ref(), code_rect, code).await?;
    tokio::time::sleep(Duration::from_millis(if is_fake { 20 } else { 200 })).await;

    let after_fill = probe_panel(page.as_ref()).await;
    let pre_submit_error = after_fill.inline_error.or(panel.inline_error);
    page.capture_buffer().clear();
    if let Some(submit_rect) = after_fill.submit_rect {
        let (cx, cy) = submit_rect.center();
        page.click_xy(cx, cy, MouseButton::Left, 1).await?;
    } else {
        page.dispatch_key(&KeyEventSpec {
            key: "Enter".to_owned(),
            code: Some("Enter".to_owned()),
            windows_virtual_key_code: Some(13),
            modifiers: KeyModifiers::default(),
            text: None,
        })
        .await?;
    }

    let capture_buf = page.capture_buffer();
    let submitted_at = Instant::now();
    let dom_error_grace = if is_fake {
        Duration::from_millis(40)
    } else {
        Duration::from_millis(1500)
    };
    let deadline = submitted_at
        + if is_fake {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(15)
        };

    while Instant::now() < deadline {
        if let Some(login) =
            check_and_settle_authenticated(&mut guard, Duration::from_secs(4)).await?
        {
            let status = guard.status().await;
            return Ok(json!({
                "status": "authenticated",
                "profile_id": profile_name,
                "login": login,
                "page_url": status.page_url,
                "checked_at": utc_now(),
            }));
        }

        let login = guard.classify_login().await?;
        if is_blocking_captcha(login.verification_kind) {
            return Ok(json!({
                "status": "verification_required",
                "profile_id": profile_name,
                "verification_kind": verification_kind_str(login.verification_kind),
                "message": "Douyin presented a slider or security challenge during SMS verification. Complete it via `doudou session login`.",
                "checked_at": utc_now(),
            }));
        }

        let panel_now = probe_panel(page.as_ref()).await;
        let telemetry = inspect_passport_capture(&capture_buf);
        let fresh_dom_error = panel_now.inline_error.filter(|msg| {
            pre_submit_error.as_deref() != Some(msg.as_str())
                || submitted_at.elapsed() >= dom_error_grace
        });
        if let Some(err_msg) = telemetry.sms_login_error.or(fresh_dom_error) {
            return Err(DoudouError::new("AUTH_SMS_VERIFY_FAILED", err_msg)
                .with_details(json!({ "profile_id": profile_name })));
        }
        tokio::time::sleep(Duration::from_millis(if is_fake { 25 } else { 400 })).await;
    }

    Err(DoudouError::new(
        "AUTH_SMS_VERIFY_FAILED",
        "Timed out waiting for SMS login confirmation on Douyin.",
    )
    .with_details(json!({ "profile_id": profile_name })))
}

/// Poll the live headless auth page until authenticated, SMS required, captcha blocked, or timed out.
pub async fn auth_wait(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile_name: &str,
    params: &Value,
) -> Result<Value> {
    let timeout_seconds = params
        .get("timeout_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(180.0)
        .clamp(1.0, 600.0);
    let auto_refresh = params
        .get("auto_refresh")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let is_fake = {
        let guard = session.lock().await;
        guard.mode() == SessionMode::Fake
    };

    let deadline = Instant::now() + Duration::from_secs_f64(timeout_seconds);
    let mut refresh_count = 0u32;

    while Instant::now() < deadline {
        let snap = auth_status(session, home, data_root, profile_name).await?;
        let state = snap
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("login_required");
        match state {
            "authenticated" | "sms_required" => return Ok(snap),
            "verification_required" => {
                return Err(DoudouError::new(
                    "VERIFICATION_REQUIRED",
                    "Headless authentication encountered a slider or passkey challenge. Use `doudou session login` in a local desktop terminal.",
                )
                .with_details(snap));
            }
            "qr_expired" => {
                if auto_refresh && refresh_count < 3 {
                    refresh_count += 1;
                    let _ = auth_refresh_qr(session, home, data_root, profile_name).await?;
                } else {
                    return Ok(snap);
                }
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(if is_fake { 25 } else { 1000 })).await;
    }

    Err(DoudouError::new(
        "AUTH_LOGIN_TIMEOUT",
        format!(
            "Headless authentication for profile `{profile_name}` did not complete within {timeout_seconds:.0}s."
        ),
    )
    .with_details(json!({
        "profile_id": profile_name,
        "timeout_seconds": timeout_seconds,
        "qr_refreshes": refresh_count,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_phone_hides_middle_digits() {
        assert_eq!(mask_phone("13812345678"), "138****5678");
        assert_eq!(mask_phone(" 138 1234 5678 "), "138****5678");
        assert_eq!(mask_phone("1234"), "****");
    }

    #[test]
    fn decode_data_image_url_handles_valid_and_invalid_inputs() {
        let png = b"\x89PNG\r\n\x1a\nhello";
        let url = format!("data:image/png;base64,{}", BASE64.encode(png));
        assert_eq!(decode_data_image_url(&url).as_deref(), Some(png.as_slice()));
        assert!(decode_data_image_url("https://example.com/a.png").is_none());
        assert!(decode_data_image_url("data:text/plain;base64,aGk=").is_none());
    }

    #[test]
    fn render_qr_terminal_builds_ansi_and_plain_half_block_art() {
        let mut matrix = vec!["0".repeat(21); 21];
        matrix[0] = "1".repeat(21);
        matrix[20] = "1".repeat(21);
        let rendered = render_qr_terminal(&matrix).expect("valid 21x21 qr");
        assert_eq!(rendered.modules, 21);
        // 21 + 4 quiet-zone rows = 25 rows -> ceil(25/2) = 13 half-block lines.
        assert_eq!(rendered.ansi.lines().count(), 13);
        assert_eq!(rendered.text.lines().count(), 13);
        assert!(
            rendered
                .ansi
                .lines()
                .all(|line| line.starts_with("\x1b[30;47m") && line.ends_with("\x1b[0m"))
        );
        assert!(
            rendered.ansi.contains('▀')
                || rendered.ansi.contains('▄')
                || rendered.ansi.contains('█')
        );
        assert!(render_qr_terminal(&vec!["10".to_owned(); 2]).is_none());
        assert!(render_qr_terminal(&vec!["1".repeat(20); 21]).is_none());
    }
}
