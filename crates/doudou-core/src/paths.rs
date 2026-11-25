//! Standard OS `AppData` profile paths and project-local business record paths.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::{DoudouError, Result};
use crate::io::{absolutize_path, expand_tilde, safe_segment};

pub const CURRENT_PROFILE_SCHEMA: &str = "doudou.profile.v3";
pub const CURRENT_PROFILE_RUNTIME: &str = "chrome-rust";
pub const CURRENT_IDENTITY_SCHEMA: &str = "doudou.user-identity.v1";

const IDENTITY_KEYS: &[&str] = &[
    "sender",
    "author",
    "user",
    "mention",
    "mentions",
    "reference_user",
    "source_users",
    "joined_users",
    "participants",
    "members",
    "mentioned_users",
];

const IDENTITY_CONTAINER_KEYS: &[&str] = &[
    "candidates",
    "comments",
    "conversations",
    "messages",
    "records",
    "replies",
    "reply",
    "videos",
];

/// Resolve the user-level Doudou home directory holding browser profiles and broker state.
///
/// Priority:
/// 1. Explicit `override_home` parameter (e.g. CLI `--home`).
/// 2. `DOUDOU_HOME` environment variable.
/// 3. OS-standard application data directory:
///    - Windows: `%LOCALAPPDATA%\doudou` (`~/AppData/Local/doudou`)
///    - macOS: `~/Library/Application Support/doudou`
///    - Linux: `${XDG_DATA_HOME:-~/.local/share}/doudou`
pub fn doudou_home(override_home: Option<&Path>) -> PathBuf {
    if let Some(explicit) = override_home {
        return absolutize_path(&expand_tilde(explicit));
    }
    if let Ok(configured) = std::env::var("DOUDOU_HOME")
        && !configured.trim().is_empty()
    {
        let candidate = expand_tilde(Path::new(configured.trim()));
        if candidate.is_absolute() {
            return absolutize_path(&candidate);
        }
        if let Some(home) = dirs::home_dir() {
            return absolutize_path(&home.join(candidate));
        }
        return absolutize_path(&candidate);
    }
    if let Some(base) = dirs::data_local_dir() {
        return absolutize_path(&base.join("doudou"));
    }
    if let Some(home) = dirs::home_dir() {
        return absolutize_path(&home.join("appdata").join("doudou"));
    }
    absolutize_path(Path::new(".doudou"))
}

/// Directory containing all named browser profiles under `doudou_home`.
pub fn profiles_root(home: &Path) -> PathBuf {
    home.join("profiles")
}

/// Directory containing active broker registrations under `doudou_home`.
pub fn broker_registry_root(home: &Path) -> PathBuf {
    home.join("brokers")
}

/// All resolved filesystem locations for one named profile and project data root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilePaths {
    pub home: PathBuf,
    pub data_root: PathBuf,
    pub profile_id: String,
    pub profile: PathBuf,
    pub browser_data: PathBuf,
    pub metadata: PathBuf,
    pub browser_config: PathBuf,
    pub lock: PathBuf,
    pub runtime: PathBuf,
    pub broker_descriptor: PathBuf,
    pub broker_token: PathBuf,
    pub broker_stdout: PathBuf,
    pub broker_stderr: PathBuf,
    pub message: PathBuf,
    pub conversations: PathBuf,
    pub session_index: PathBuf,
    pub runs: PathBuf,
    pub plans: PathBuf,
    pub sends: PathBuf,
    pub interactions: PathBuf,
    pub diagnostics: PathBuf,
    pub identities: PathBuf,
}

fn build_profile_paths(home: &Path, data_root: &Path, profile_id: String) -> ProfilePaths {
    let profile = profiles_root(home).join(&profile_id);
    let runtime = profile.join("runtime");
    let message = data_root.join("message").join("profiles").join(&profile_id);
    ProfilePaths {
        home: home.to_path_buf(),
        data_root: data_root.to_path_buf(),
        profile_id,
        browser_data: profile.join("browser-data"),
        metadata: profile.join("profile.json"),
        browser_config: profile.join("chrome-config.json"),
        lock: profile.join(".lock.json"),
        broker_descriptor: runtime.join("broker.json"),
        broker_token: runtime.join("broker.token"),
        broker_stdout: runtime.join("broker.stdout.log"),
        broker_stderr: runtime.join("broker.stderr.log"),
        runtime,
        profile,
        conversations: message.join("conversations"),
        session_index: message.join("session-index"),
        runs: message.join("runs"),
        plans: message.join("plans"),
        sends: message.join("sends"),
        interactions: message.join("interactions"),
        diagnostics: message.join("diagnostics"),
        identities: message.join("identities"),
        message,
    }
}

/// Compute paths for a user-supplied profile name.
pub fn profile_paths(home: &Path, data_root: &Path, profile_name: &str) -> Result<ProfilePaths> {
    let profile_id = safe_segment(profile_name, 96)?;
    Ok(build_profile_paths(home, data_root, profile_id))
}

/// Compute paths for an already-stored `profile_id` directory name.
pub fn stored_profile_paths(
    home: &Path,
    data_root: &Path,
    profile_id: &str,
) -> Result<ProfilePaths> {
    if profile_id.is_empty()
        || matches!(profile_id, "." | "..")
        || Path::new(profile_id).file_name().and_then(|n| n.to_str()) != Some(profile_id)
    {
        return Err(DoudouError::new(
            "INVALID_PATH_SEGMENT",
            "Invalid stored profile id.",
        ));
    }
    Ok(build_profile_paths(home, data_root, profile_id.to_owned()))
}

fn legacy_unsupported(legacy_format: &str, path: &str, supported_path: &str) -> DoudouError {
    DoudouError::new(
        "LEGACY_UNSUPPORTED",
        "Legacy Doudou data is not supported by this version.",
    )
    .with_details(json!({
        "legacy_format": legacy_format,
        "path": path,
        "supported_path": supported_path,
    }))
}

/// Check whether a profile metadata JSON object matches the current Chrome/Rust schema.
pub fn is_current_profile_metadata(metadata: &Value) -> bool {
    metadata.get("schema").and_then(Value::as_str) == Some(CURRENT_PROFILE_SCHEMA)
        && metadata.get("runtime").and_then(Value::as_str) == Some(CURRENT_PROFILE_RUNTIME)
}

/// Validate profile metadata schema and runtime, returning `LEGACY_UNSUPPORTED` on mismatch.
pub fn assert_supported_profile_metadata(metadata: &Value) -> Result<()> {
    if !is_current_profile_metadata(metadata) {
        return Err(legacy_unsupported(
            "browser_profile",
            "profiles/<PROFILE>/profile.json",
            &format!("{CURRENT_PROFILE_SCHEMA}:{CURRENT_PROFILE_RUNTIME}"),
        ));
    }
    Ok(())
}

/// Initialize the user profile root and project message root, rejecting legacy `Profile/` casing.
pub fn initialize_roots(home: &Path, data_root: &Path) -> Result<()> {
    if has_exact_child(data_root, "Profile") {
        return Err(legacy_unsupported("profile_root", "Profile/", "profiles/"));
    }
    for dir in [
        profiles_root(home),
        broker_registry_root(home),
        data_root.join("message"),
    ] {
        fs::create_dir_all(&dir).map_err(|err| {
            DoudouError::new(
                "IO_ERROR",
                format!("Failed creating directory {}: {err}", dir.display()),
            )
        })?;
    }
    Ok(())
}

fn has_exact_child(parent: &Path, name: &str) -> bool {
    let Ok(entries) = fs::read_dir(parent) else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| entry.file_name().to_str() == Some(name))
}

fn has_legacy_identity(value: &Value, identity_context: bool) -> bool {
    match value {
        Value::Array(items) => items
            .iter()
            .any(|item| has_legacy_identity(item, identity_context)),
        Value::Object(map) => {
            if identity_context {
                let has_any_id = ["uid", "sec_uid", "unique_id", "secUid"].iter().any(|key| {
                    map.get(*key)
                        .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
                });
                if has_any_id
                    && map.get("schema").and_then(Value::as_str) != Some(CURRENT_IDENTITY_SCHEMA)
                {
                    return true;
                }
            }
            for key in IDENTITY_KEYS {
                if let Some(child) = map.get(*key)
                    && has_legacy_identity(child, true)
                {
                    return true;
                }
            }
            for key in IDENTITY_CONTAINER_KEYS {
                if let Some(child) = map.get(*key)
                    && has_legacy_identity(child, false)
                {
                    return true;
                }
            }
            if let Some(sys) = map.get("system_event")
                && sys.is_object()
                && has_legacy_identity(sys, false)
            {
                return true;
            }
            if let Some(props) = map.get("property_list")
                && props.is_object()
                && has_legacy_identity(props, true)
            {
                return true;
            }
            false
        }
        _ => false,
    }
}

fn file_has_legacy_identity(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    if path.extension().and_then(|e| e.to_str()) == Some("json") {
        return serde_json::from_str::<Value>(&content)
            .is_ok_and(|v| has_legacy_identity(&v, false));
    }
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .any(|line| {
            serde_json::from_str::<Value>(line).is_ok_and(|v| has_legacy_identity(&v, false))
        })
}

/// Scan stored message records for legacy unversioned identity objects.
pub fn assert_supported_identity_records(
    home: &Path,
    data_root: &Path,
    profile_name: &str,
) -> Result<()> {
    let message_root = profile_paths(home, data_root, profile_name)?.message;
    if !message_root.is_dir() {
        return Ok(());
    }
    let mut stack = vec![message_root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("json" | "jsonl")
            ) && file_has_legacy_identity(&path)
            {
                return Err(legacy_unsupported(
                    "identity_record",
                    "message/profiles/<PROFILE>/<RECORD>",
                    CURRENT_IDENTITY_SCHEMA,
                ));
            }
        }
    }
    Ok(())
}
