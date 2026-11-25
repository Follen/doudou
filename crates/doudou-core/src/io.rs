//! Time, hashing, path-segment encoding, atomic filesystem I/O, and NDJSON emission.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rand::Rng as _;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tempfile::Builder;
use unicode_normalization::UnicodeNormalization as _;

use crate::error::{DoudouError, Result};

/// Return the current UTC timestamp formatted with millisecond precision (`Z` suffix).
pub fn utc_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Generate a time-ordered random identifier matching `<prefix>-YYYYMMDDHHMMSS-<8hex>`.
pub fn random_id(prefix: &str) -> String {
    let stamp = Utc::now().format("%Y%m%d%H%M%S");
    let mut bytes = [0u8; 4];
    rand::rng().fill(&mut bytes);
    format!("{prefix}-{stamp}-{}", hex::encode(bytes))
}

/// Compute lowercase hex SHA-256 of raw bytes.
pub fn sha256_bytes(value: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value);
    hex::encode(hasher.finalize())
}

/// Compute lowercase hex SHA-256 of UTF-8 text.
pub fn sha256_str(value: &str) -> String {
    sha256_bytes(value.as_bytes())
}

/// Encode an arbitrary identifier into a portable, collision-resistant path segment.
pub fn safe_segment(value: &str, max_length: usize) -> Result<String> {
    let normalized: String = value.nfkc().collect::<String>().trim().to_owned();
    if normalized.is_empty() {
        return Err(DoudouError::new(
            "INVALID_PATH_SEGMENT",
            "A non-empty path value is required.",
        ));
    }
    let mut encoded = String::with_capacity(normalized.len());
    for ch in normalized.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') || ch.is_alphanumeric() {
            encoded.push(ch);
        } else {
            let code = ch as u32;
            let _ = write!(encoded, "~{code:04X}");
        }
    }
    if encoded.chars().all(|c| c == '.') {
        encoded = format!("~{}", hex::encode(encoded.as_bytes()));
    }
    if encoded.chars().count() > max_length {
        let prefix_len = max_length.saturating_sub(13);
        let prefix: String = encoded.chars().take(prefix_len).collect();
        let digest = sha256_str(&normalized);
        encoded = format!("{prefix}~{}", &digest[..12]);
    }
    Ok(encoded)
}

/// Normalize text for case-insensitive, whitespace-collapsed lookup.
pub fn normalize_lookup(value: &str) -> String {
    let nfkc: String = value.nfkc().collect();
    nfkc.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Resolve the project data root directory, defaulting to the current working directory.
pub fn resolve_data_root(candidate: Option<&Path>) -> Result<PathBuf> {
    let path = match candidate {
        Some(p) => expand_tilde(p),
        None => std::env::current_dir().map_err(|err| {
            DoudouError::new(
                "IO_ERROR",
                format!("Failed to resolve current directory: {err}"),
            )
        })?,
    };
    Ok(absolutize_path(&path))
}

/// Expand a leading `~` against the user's home directory.
pub fn expand_tilde(path: &Path) -> PathBuf {
    if let Some(text) = path.to_str()
        && let Some(home) = dirs::home_dir()
    {
        if text == "~" {
            return home;
        }
        if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

/// Convert a path to an absolute lexical path without requiring it to exist yet.
pub fn absolutize_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return strip_windows_verbatim(canonical);
    }
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(path)
    } else {
        path.to_path_buf()
    };
    normalize_components(&joined)
}

fn normalize_components(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(windows)]
fn strip_windows_verbatim(path: PathBuf) -> PathBuf {
    if let Some(s) = path.to_str()
        && let Some(stripped) = s.strip_prefix(r"\\?\")
        && !stripped.starts_with("UNC\\")
    {
        return PathBuf::from(stripped);
    }
    path
}

#[cfg(not(windows))]
fn strip_windows_verbatim(path: PathBuf) -> PathBuf {
    path
}

/// Atomically write raw bytes to `path` via a sibling temporary file, fsync, and rename.
pub fn atomic_write_bytes(path: &Path, value: &[u8], unix_mode: Option<u32>) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        DoudouError::new(
            "IO_ERROR",
            format!("Path has no parent directory: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent).map_err(|err| {
        DoudouError::new(
            "IO_ERROR",
            format!("Failed to create directory {}: {err}", parent.display()),
        )
    })?;
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp");
    let mut tmp = Builder::new()
        .prefix(&format!(".{file_name}."))
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|err| {
            DoudouError::new(
                "IO_ERROR",
                format!("Failed to create temp file in {}: {err}", parent.display()),
            )
        })?;
    tmp.write_all(value).map_err(|err| {
        DoudouError::new(
            "IO_ERROR",
            format!("Failed writing {}: {err}", path.display()),
        )
    })?;
    tmp.flush().map_err(|err| {
        DoudouError::new(
            "IO_ERROR",
            format!("Failed flushing {}: {err}", path.display()),
        )
    })?;
    tmp.as_file().sync_all().map_err(|err| {
        DoudouError::new(
            "IO_ERROR",
            format!("Failed fsyncing {}: {err}", path.display()),
        )
    })?;

    #[cfg(unix)]
    if let Some(mode) = unix_mode {
        use std::os::unix::fs::PermissionsExt as _;
        let perms = fs::Permissions::from_mode(mode);
        fs::set_permissions(tmp.path(), perms).map_err(|err| {
            DoudouError::new(
                "IO_ERROR",
                format!("Failed chmod {}: {err}", path.display()),
            )
        })?;
    }
    #[cfg(not(unix))]
    let _ = unix_mode;

    let tmp_path = tmp.into_temp_path();
    persist_replace(tmp_path, path)
}

fn persist_replace(tmp_path: tempfile::TempPath, dest: &Path) -> Result<()> {
    match tmp_path.persist(dest) {
        Ok(()) => Ok(()),
        Err(err) => {
            #[cfg(windows)]
            {
                if dest.exists() {
                    let _ = fs::remove_file(dest);
                    if err.path.persist(dest).is_ok() {
                        return Ok(());
                    }
                }
            }
            Err(DoudouError::new(
                "IO_ERROR",
                format!("Failed atomic replace {}: {}", dest.display(), err.error),
            ))
        }
    }
}

/// Atomically write UTF-8 text to `path`.
pub fn atomic_write_text(path: &Path, value: &str, unix_mode: Option<u32>) -> Result<()> {
    atomic_write_bytes(path, value.as_bytes(), unix_mode)
}

/// Atomically write pretty-printed JSON with a trailing newline to `path`.
pub fn atomic_write_json<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    unix_mode: Option<u32>,
) -> Result<()> {
    let mut rendered = serde_json::to_string_pretty(value).map_err(|err| {
        DoudouError::new(
            "JSON_SERIALIZE_ERROR",
            format!("Failed to encode JSON: {err}"),
        )
    })?;
    rendered.push('\n');
    atomic_write_text(path, &rendered, unix_mode)
}

/// Atomically write compact JSONL records to `path`.
pub fn atomic_write_jsonl<T: Serialize>(path: &Path, values: &[T]) -> Result<()> {
    let mut out = String::new();
    for item in values {
        let line = serde_json::to_string(item).map_err(|err| {
            DoudouError::new(
                "JSON_SERIALIZE_ERROR",
                format!("Failed to encode JSONL: {err}"),
            )
        })?;
        out.push_str(&line);
        out.push('\n');
    }
    atomic_write_text(path, &out, None)
}

/// Read a JSON document from `path`. When `optional` is true, returns `Ok(None)` if missing.
pub fn read_json(path: &Path, optional: bool) -> Result<Option<Value>> {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).map(Some).map_err(|err| {
            DoudouError::new(
                "INVALID_JSON",
                format!("Invalid JSON in {}.", path.display()),
            )
            .with_details(serde_json::json!({
                "path": path.display().to_string(),
                "message": err.to_string(),
            }))
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && optional => Ok(None),
        Err(err) => Err(DoudouError::new(
            "IO_ERROR",
            format!("Failed reading {}: {err}", path.display()),
        )),
    }
}

/// Read JSONL records from `path`, enforcing a complete trailing newline.
pub fn read_jsonl(path: &Path, optional: bool) -> Result<Vec<Value>> {
    let content = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && optional => {
            return Ok(Vec::new());
        }
        Err(err) => {
            return Err(DoudouError::new(
                "IO_ERROR",
                format!("Failed reading {}: {err}", path.display()),
            ));
        }
    };
    if !content.is_empty() && !content.ends_with('\n') {
        return Err(DoudouError::new(
            "INCOMPLETE_JSONL_LINE",
            format!(
                "JSONL file has an incomplete final line: {}.",
                path.display()
            ),
        )
        .with_details(serde_json::json!({ "path": path.display().to_string() })));
    }
    let mut records = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if line.is_empty() {
            continue;
        }
        let parsed: Value = serde_json::from_str(line).map_err(|_| {
            DoudouError::new(
                "INVALID_JSONL",
                format!("Invalid JSONL at {}:{line_no}.", path.display()),
            )
            .with_details(serde_json::json!({
                "path": path.display().to_string(),
                "line": line_no,
            }))
        })?;
        records.push(parsed);
    }
    Ok(records)
}

/// Emit one compact NDJSON event line to stdout and flush immediately.
pub fn emit(value: &Value) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    if let Ok(line) = serde_json::to_string(value) {
        let _ = writeln!(handle, "{line}");
        let _ = handle.flush();
    }
}

/// Touch or ensure a parent directory is writable by creating and deleting a probe file.
pub fn is_dir_writable(dir: &Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".doudou-write-probe-{}", std::process::id()));
    let ok = File::create(&probe).is_ok();
    let _ = fs::remove_file(&probe);
    ok
}
