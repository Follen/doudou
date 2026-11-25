//! Cross-platform process identity and liveness inspection.

use std::process::Command;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Tolerance window in seconds when comparing process creation timestamps across queries.
const START_TIME_TOLERANCE_SECONDS: f64 = 2.0;

/// OS process identity snapshot used to detect PID reuse across lock and broker files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    /// Operating system process identifier.
    pub pid: u32,
    /// UTC ISO-8601 process creation timestamp when available from the OS.
    pub start: Option<String>,
    /// Optional command line string when available.
    pub command: Option<String>,
}

impl ProcessIdentity {
    /// Construct a process identity with explicit fields.
    pub fn new(pid: u32, start: Option<String>, command: Option<String>) -> Self {
        Self {
            pid,
            start,
            command,
        }
    }
}

/// Parse UTF-8 JSON emitted by the Windows `Get-CimInstance Win32_Process` helper.
pub fn parse_windows_process_json(stdout: &str) -> Option<ProcessIdentity> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed: Value = serde_json::from_str(trimmed).ok()?;
    let pid = parsed
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())?;
    if pid == 0 {
        return None;
    }
    let start = parsed
        .get("start")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let command = parsed
        .get("command")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    Some(ProcessIdentity {
        pid,
        start,
        command,
    })
}

/// Build the Windows `PowerShell` command script used to inspect `pid` with pinned UTF-8 output.
pub fn windows_process_query_script(pid: u32) -> String {
    format!(
        "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); \
         $p = Get-CimInstance Win32_Process -Filter \"ProcessId = {pid}\"; \
         if ($null -eq $p) {{ exit 3 }}; \
         $o = [ordered]@{{ pid=[int]$p.ProcessId; start=[string]$p.CreationDate.ToUniversalTime().ToString(\"o\"); command=[string]$p.CommandLine }}; \
         $o | ConvertTo-Json -Compress"
    )
}

/// Return `true` when a Linux `/proc/<pid>/stat` line indicates a zombie (`'Z'`) process.
pub fn proc_stat_is_zombie(stat_content: &str) -> bool {
    let Some(close_paren) = stat_content.rfind(')') else {
        return false;
    };
    let rest = stat_content.get(close_paren + 2..).unwrap_or_default();
    rest.split_whitespace().next() == Some("Z")
}

/// Return a cached [`ProcessIdentity`] for the current running process.
pub fn current_process_identity() -> ProcessIdentity {
    static CURRENT: OnceLock<ProcessIdentity> = OnceLock::new();
    CURRENT
        .get_or_init(|| {
            let pid = std::process::id();
            query_process_identity_uncached(pid).unwrap_or_else(|| ProcessIdentity {
                pid,
                start: Some(doudou_core::utc_now()),
                command: None,
            })
        })
        .clone()
}

/// Inspect the OS for a live process with identifier `pid`.
///
/// Returns `None` when `pid == 0`, the process does not exist, or it is a Linux zombie.
pub fn process_identity(pid: u32) -> Option<ProcessIdentity> {
    if pid == 0 {
        return None;
    }
    if pid == std::process::id() {
        return Some(current_process_identity());
    }
    query_process_identity_uncached(pid)
}

#[cfg(windows)]
fn query_process_identity_uncached(pid: u32) -> Option<ProcessIdentity> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let script = windows_process_query_script(pid);
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_windows_process_json(&stdout)
}

#[cfg(not(windows))]
fn query_process_identity_uncached(pid: u32) -> Option<ProcessIdentity> {
    use std::fs;
    use std::path::Path;

    let proc_dir = format!("/proc/{pid}");
    let proc_path = Path::new(&proc_dir);
    if proc_path.exists() {
        let stat_text = fs::read_to_string(proc_path.join("stat")).ok()?;
        if proc_stat_is_zombie(&stat_text) {
            return None;
        }
        let start = fs::metadata(proc_path)
            .and_then(|meta| meta.modified().or_else(|_| meta.created()))
            .ok()
            .map(|ts| {
                let dt: DateTime<Utc> = ts.into();
                dt.to_rfc3339()
            });
        return Some(ProcessIdentity {
            pid,
            start,
            command: None,
        });
    }

    // Fallback for Unix platforms without /proc (e.g. macOS): probe via `kill -0`.
    let status = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    Some(ProcessIdentity {
        pid,
        start: None,
        command: None,
    })
}

/// Compare two process identities, accounting for PID reuse via start timestamps within 2 seconds.
pub fn same_process(expected: Option<&ProcessIdentity>, actual: Option<&ProcessIdentity>) -> bool {
    let (Some(left), Some(right)) = (expected, actual) else {
        return false;
    };
    if left.pid != right.pid {
        return false;
    }
    let (Some(left_start), Some(right_start)) = (&left.start, &right.start) else {
        return true;
    };
    let Ok(left_dt) = DateTime::parse_from_rfc3339(left_start) else {
        return false;
    };
    let Ok(right_dt) = DateTime::parse_from_rfc3339(right_start) else {
        return false;
    };
    let delta = left_dt
        .with_timezone(&Utc)
        .signed_duration_since(right_dt.with_timezone(&Utc));
    let seconds = delta.num_milliseconds().abs() as f64 / 1000.0;
    seconds < START_TIME_TOLERANCE_SECONDS
}

/// Return `true` when the recorded `pid` and optional `process_start` match a live OS process.
pub fn is_recorded_process_live(pid: u32, process_start: Option<&str>) -> bool {
    if pid == 0 {
        return false;
    }
    let expected = ProcessIdentity {
        pid,
        start: process_start.map(ToOwned::to_owned),
        command: None,
    };
    let actual = process_identity(pid);
    same_process(Some(&expected), actual.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_process_json_parses_utf8_and_handles_empty() {
        assert!(parse_windows_process_json("").is_none());
        assert!(parse_windows_process_json("   ").is_none());
        assert!(parse_windows_process_json("not json").is_none());

        let parsed = parse_windows_process_json(
            r#"{"pid":123,"start":"2026-08-04T00:00:00.0000000Z","command":"中文命令"}"#,
        )
        .expect("valid process json");
        assert_eq!(parsed.pid, 123);
        assert_eq!(parsed.command.as_deref(), Some("中文命令"));
    }

    #[test]
    fn windows_query_script_pins_utf8_encoding() {
        let script = windows_process_query_script(42);
        assert!(script.contains("UTF8Encoding]::new($false)"));
        assert!(script.contains("ProcessId = 42"));
    }

    #[test]
    fn same_process_handles_tolerance_and_pid_reuse() {
        let a = ProcessIdentity::new(100, Some("2026-11-25T10:00:00.000Z".to_owned()), None);
        let b = ProcessIdentity::new(100, Some("2026-11-25T10:00:01.500Z".to_owned()), None);
        let c = ProcessIdentity::new(100, Some("2026-11-25T10:00:05.000Z".to_owned()), None);
        let d = ProcessIdentity::new(101, Some("2026-11-25T10:00:00.000Z".to_owned()), None);

        assert!(same_process(Some(&a), Some(&b)));
        assert!(!same_process(Some(&a), Some(&c)));
        assert!(!same_process(Some(&a), Some(&d)));
        assert!(!same_process(Some(&a), None));
    }

    #[test]
    fn proc_stat_detects_zombies() {
        assert!(proc_stat_is_zombie("1234 (chrome (1)) Z 1 2 3"));
        assert!(!proc_stat_is_zombie("1234 (chrome) S 1 2 3"));
    }
}
