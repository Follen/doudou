//! Profile lifecycle CRUD, `chrome-config.json` defaults, and profile summary listing.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};
use doudou_core::{
    CURRENT_PROFILE_RUNTIME, CURRENT_PROFILE_SCHEMA, DoudouError, ProfilePaths, Result,
    assert_supported_profile_metadata, atomic_write_json, initialize_roots,
    is_current_profile_metadata, profile_paths, profiles_root, read_json, stored_profile_paths,
    utc_now,
};
use rand::Rng as _;
use serde_json::{Map, Value, json};

/// Schema identifier for Chrome browser configuration files stored beside `profile.json`.
pub const CHROME_CONFIG_SCHEMA: &str = "doudou.chrome-config.v1";

/// Controls whether deleting a profile removes only browser state or also project message records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeleteScope {
    /// Remove `profiles/<profile_id>` under `doudou_home`, keeping project-local messages.
    ProfileOnly,
    /// Remove both `profiles/<profile_id>` and `message/profiles/<profile_id>`.
    ProfileAndMessages,
}

/// Build default per-profile `chrome-config.json` with deterministic locale/timezone and random seeds.
pub fn default_browser_config(profile_id: &str) -> Value {
    let mut rng = rand::rng();
    let timestamp = utc_now();
    let target_os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    json!({
        "schema": CHROME_CONFIG_SCHEMA,
        "profile_id": profile_id,
        "version": 1,
        "target_os": target_os,
        "locale": "zh-CN",
        "timezone": "Asia/Shanghai",
        "config": {
            "fonts:spacing_seed": rng.random_range(1u32..=4_294_967_294u32),
            "audio:seed": rng.random_range(1u32..=4_294_967_294u32),
            "canvas:seed": rng.random_range(1u32..=4_294_967_294u32),
        },
        "generated_fingerprint": Value::Null,
        "created_at": timestamp,
        "updated_at": timestamp,
    })
}

/// Create a new profile under OS `AppData` (`home`) and project message directories (`data_root`).
pub fn create_profile(
    home: &Path,
    data_root: &Path,
    profile_name: &str,
    display_name: Option<&str>,
) -> Result<(Value, ProfilePaths)> {
    let paths = profile_paths(home, data_root, profile_name)?;
    if paths.metadata.exists() {
        if let Some(existing) = read_json(&paths.metadata, true)? {
            assert_supported_profile_metadata(&existing)?;
        }
        return Err(DoudouError::new(
            "PROFILE_EXISTS",
            format!("Profile already exists: {profile_name}."),
        )
        .with_details(json!({ "profile_id": paths.profile_id })));
    }
    initialize_roots(home, data_root)?;
    for directory in [
        &paths.browser_data,
        &paths.runtime,
        &paths.conversations,
        &paths.session_index,
        &paths.runs,
        &paths.plans,
        &paths.sends,
        &paths.interactions,
        &paths.diagnostics,
        &paths.identities,
    ] {
        fs::create_dir_all(directory).map_err(|err| {
            DoudouError::new(
                "IO_ERROR",
                format!("Failed creating directory {}: {err}", directory.display()),
            )
        })?;
    }
    let timestamp = utc_now();
    let metadata = json!({
        "schema": CURRENT_PROFILE_SCHEMA,
        "profile_id": paths.profile_id,
        "display_name": display_name.unwrap_or(profile_name),
        "runtime": CURRENT_PROFILE_RUNTIME,
        "browser_data_dir": "browser-data",
        "browser_config": "chrome-config.json",
        "created_at": timestamp,
        "updated_at": timestamp,
        "last_login_verified_at": Value::Null,
        "login_status": "login_required",
        "last_error_code": Value::Null,
    });
    atomic_write_json(
        &paths.browser_config,
        &default_browser_config(&paths.profile_id),
        Some(0o600),
    )?;
    atomic_write_json(&paths.metadata, &metadata, None)?;
    Ok((metadata, paths))
}

/// Load and validate an existing profile's metadata and resolved filesystem paths.
pub fn load_profile(
    home: &Path,
    data_root: &Path,
    profile_name: &str,
) -> Result<(Value, ProfilePaths)> {
    let paths = profile_paths(home, data_root, profile_name)?;
    let Some(metadata) = read_json(&paths.metadata, true)? else {
        return Err(DoudouError::new(
            "PROFILE_NOT_FOUND",
            format!("Profile not found: {profile_name}."),
        )
        .with_details(json!({ "profile_id": paths.profile_id })));
    };
    assert_supported_profile_metadata(&metadata)?;
    initialize_roots(home, data_root)?;
    if metadata.get("profile_id").and_then(Value::as_str) != Some(paths.profile_id.as_str()) {
        return Err(DoudouError::new(
            "PROFILE_METADATA_MISMATCH",
            "Profile metadata does not match its directory.",
        )
        .with_details(json!({ "path": paths.metadata.display().to_string() })));
    }
    Ok((metadata, paths))
}

/// Merge `patch` fields into a profile's metadata, bump `updated_at`, and persist atomically.
pub fn update_profile(
    home: &Path,
    data_root: &Path,
    profile_name: &str,
    patch: &Map<String, Value>,
) -> Result<Value> {
    let (mut metadata, paths) = load_profile(home, data_root, profile_name)?;
    let obj = metadata.as_object_mut().ok_or_else(|| {
        DoudouError::new("INVALID_JSON", "Profile metadata must be a JSON object.")
    })?;
    for (key, value) in patch {
        obj.insert(key.clone(), value.clone());
    }
    obj.insert("updated_at".to_owned(), json!(utc_now()));
    atomic_write_json(&paths.metadata, &metadata, None)?;
    Ok(metadata)
}

/// Delete a validated profile directory and optionally its project-local message directory.
pub fn delete_profile(
    home: &Path,
    data_root: &Path,
    profile_name: &str,
    scope: DeleteScope,
) -> Result<Value> {
    let (_metadata, paths) = load_profile(home, data_root, profile_name)?;
    if paths.profile.exists() {
        fs::remove_dir_all(&paths.profile).map_err(|err| {
            DoudouError::new(
                "IO_ERROR",
                format!(
                    "Failed deleting profile directory {}: {err}",
                    paths.profile.display()
                ),
            )
        })?;
    }
    let deleted_message_dir = if scope == DeleteScope::ProfileAndMessages && paths.message.exists()
    {
        fs::remove_dir_all(&paths.message).map_err(|err| {
            DoudouError::new(
                "IO_ERROR",
                format!(
                    "Failed deleting message directory {}: {err}",
                    paths.message.display()
                ),
            )
        })?;
        true
    } else {
        false
    };
    Ok(json!({
        "profile_id": paths.profile_id,
        "deleted_profile_dir": true,
        "deleted_message_dir": deleted_message_dir,
    }))
}

/// List all current-schema profiles with non-sensitive runtime, chat, content, and write summaries.
///
/// The `is_lock_live` closure probes each profile's `.lock.json` path and returns `true` when an
/// active process currently owns the profile lock.
pub fn list_profiles<F>(home: &Path, data_root: &Path, mut is_lock_live: F) -> Result<Vec<Value>>
where
    F: FnMut(&Path) -> bool,
{
    initialize_roots(home, data_root)?;
    let root = profiles_root(home);
    let entries = fs::read_dir(&root).map_err(|err| {
        DoudouError::new(
            "IO_ERROR",
            format!(
                "Failed reading profiles directory {}: {err}",
                root.display()
            ),
        )
    })?;

    let mut profiles: Vec<Value> = Vec::new();
    for entry in entries.flatten() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }
        let Some(metadata) = read_json(&entry_path.join("profile.json"), true)? else {
            continue;
        };
        if !is_current_profile_metadata(&metadata) {
            continue;
        }
        let Some(profile_id) = metadata.get("profile_id").and_then(Value::as_str) else {
            continue;
        };
        if entry.file_name().to_str() != Some(profile_id) {
            continue;
        }
        let paths = stored_profile_paths(home, data_root, profile_id)?;
        let conversation_index = read_json(&paths.message.join("conversations.json"), true)?;
        let session_index = read_json(&paths.session_index.join("latest.json"), true)?;
        let content_dir = paths.message.join("content");
        let users_dir = content_dir.join("users");
        let videos_dir = content_dir.join("videos");

        let conversation_count = conversation_index
            .as_ref()
            .and_then(|v| v.get("count"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let stored_conversation_count = count_subdirs(&paths.conversations, Some("media"));
        let user_count = count_subdirs(&users_dir, None);
        let video_count = count_subdirs(&videos_dir, None);
        let last_session_index_refresh = session_index
            .as_ref()
            .and_then(|v| v.get("completed_at"))
            .cloned()
            .unwrap_or(Value::Null);
        let recent_write_at = latest_receipt_timestamp(&paths).map_or(Value::Null, Value::String);

        let mut summary = metadata.as_object().cloned().unwrap_or_default();
        summary.insert(
            "lock_status".to_owned(),
            json!(if is_lock_live(&paths.lock) {
                "in_use"
            } else {
                "free"
            }),
        );
        summary.insert(
            "last_session_index_refresh".to_owned(),
            last_session_index_refresh,
        );
        summary.insert(
            "chat_summary".to_owned(),
            json!({
                "conversation_count": conversation_count,
                "stored_conversation_count": stored_conversation_count,
            }),
        );
        summary.insert(
            "content_summary".to_owned(),
            json!({
                "user_count": user_count,
                "video_count": video_count,
            }),
        );
        summary.insert("recent_write_at".to_owned(), recent_write_at);
        profiles.push(Value::Object(summary));
    }

    profiles.sort_by(|left, right| {
        let key = |item: &Value| {
            (
                item.get("created_at")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                item.get("profile_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            )
        };
        key(left).cmp(&key(right))
    });
    Ok(profiles)
}

/// Return the default background browser execution mode for the host operating system.
pub const fn platform_background_mode() -> &'static str {
    if cfg!(windows) {
        "windows_headless"
    } else if cfg!(target_os = "linux") {
        "linux_virtual_headed"
    } else {
        "headed"
    }
}

fn count_subdirs(dir: &Path, skip_name: Option<&str>) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            entry.path().is_dir()
                && skip_name.is_none_or(|skip| entry.file_name().to_str() != Some(skip))
        })
        .count()
}

fn latest_receipt_timestamp(paths: &ProfilePaths) -> Option<String> {
    let mut latest: Option<SystemTime> = None;
    collect_receipt_mtimes(&paths.sends, 2, &mut latest);
    collect_receipt_mtimes(&paths.interactions, 3, &mut latest);
    latest.map(|time| {
        let dt: DateTime<Utc> = time.into();
        dt.to_rfc3339_opts(SecondsFormat::Millis, true)
    })
}

fn collect_receipt_mtimes(dir: &Path, remaining_depth: usize, latest: &mut Option<SystemTime>) {
    if remaining_depth == 0 {
        let receipt = dir.join("receipt.json");
        if let Ok(meta) = fs::metadata(receipt)
            && let Ok(modified) = meta.modified()
            && latest.is_none_or(|current| modified > current)
        {
            *latest = Some(modified);
        }
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_receipt_mtimes(&path, remaining_depth - 1, latest);
        }
    }
}
