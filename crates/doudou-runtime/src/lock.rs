//! Atomic exclusive profile lock (`doudou.profile-lock.v2`) with stale-lock recovery.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

use doudou_core::{DoudouError, Result, read_json, utc_now};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::process::{ProcessIdentity, current_process_identity, process_identity, same_process};

/// Schema version written into `.lock.json` files.
pub const PROFILE_LOCK_SCHEMA: &str = "doudou.profile-lock.v2";

/// Maximum stale-lock recovery retries before failing with `PROFILE_IN_USE`.
const MAX_ACQUIRE_ATTEMPTS: usize = 3;

/// Metadata persisted inside a profile's `.lock.json` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockOwner {
    /// Lock schema identifier (`doudou.profile-lock.v2`).
    pub schema: String,
    /// Owning profile identifier.
    pub profile_id: String,
    /// Owning project data root path.
    pub data_root: String,
    /// Owning OS process ID.
    pub pid: u32,
    /// Owning process creation timestamp for PID-reuse detection.
    pub process_start: Option<String>,
    /// Logical owner command label (e.g. `"broker"`).
    pub command: String,
    /// UTC timestamp when the lock was created.
    pub created_at: String,
}

/// Single-owner filesystem lock protecting a Doudou profile and its Chrome user-data directory.
#[derive(Debug)]
pub struct ProfileLock {
    path: PathBuf,
    profile_id: String,
    data_root: PathBuf,
    command: String,
    owner: Option<LockOwner>,
}

impl ProfileLock {
    /// Create an unacquired lock handle targeting `path`.
    pub fn new(
        path: PathBuf,
        profile_id: impl Into<String>,
        data_root: PathBuf,
        command: impl Into<String>,
    ) -> Self {
        Self {
            path,
            profile_id: profile_id.into(),
            data_root,
            command: command.into(),
            owner: None,
        }
    }

    /// Return the lock file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the currently held [`LockOwner`], if acquired.
    pub fn owner(&self) -> Option<&LockOwner> {
        self.owner.as_ref()
    }

    /// Atomically acquire the exclusive profile lock, reclaiming stale locks from dead processes.
    pub fn acquire(&mut self) -> Result<LockOwner> {
        if let Some(existing) = &self.owner {
            return Ok(existing.clone());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                DoudouError::new(
                    "IO_ERROR",
                    format!("Failed creating lock directory {}: {err}", parent.display()),
                )
            })?;
        }

        let identity = current_process_identity();
        let owner = LockOwner {
            schema: PROFILE_LOCK_SCHEMA.to_owned(),
            profile_id: self.profile_id.clone(),
            data_root: self.data_root.display().to_string(),
            pid: identity.pid,
            process_start: identity.start,
            command: self.command.clone(),
            created_at: utc_now(),
        };

        for _ in 0..MAX_ACQUIRE_ATTEMPTS {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.path)
            {
                Ok(mut file) => {
                    let serialized = serde_json::to_string_pretty(&owner).map_err(|err| {
                        DoudouError::new(
                            "JSON_SERIALIZE_ERROR",
                            format!("Failed serializing profile lock: {err}"),
                        )
                    })?;
                    file.write_all(serialized.as_bytes())
                        .and_then(|()| file.write_all(b"\n"))
                        .and_then(|()| file.sync_all())
                        .map_err(|err| {
                            let _ = fs::remove_file(&self.path);
                            DoudouError::new(
                                "IO_ERROR",
                                format!("Failed writing lock file {}: {err}", self.path.display()),
                            )
                        })?;
                    self.owner = Some(owner.clone());
                    return Ok(owner);
                }
                Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                    let existing = read_json(&self.path, true).ok().flatten();
                    if let Some(existing_obj) = existing.as_ref().and_then(Value::as_object) {
                        let existing_pid = existing_obj
                            .get("pid")
                            .and_then(Value::as_u64)
                            .and_then(|v| u32::try_from(v).ok())
                            .unwrap_or(0);
                        let existing_start = existing_obj
                            .get("process_start")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);
                        let expected = ProcessIdentity::new(existing_pid, existing_start, None);
                        let actual = process_identity(existing_pid);
                        if same_process(Some(&expected), actual.as_ref()) {
                            let details = json!({
                                "owner": {
                                    "pid": existing_obj.get("pid").cloned().unwrap_or(Value::Null),
                                    "process_start": existing_obj.get("process_start").cloned().unwrap_or(Value::Null),
                                    "command": existing_obj.get("command").cloned().unwrap_or(Value::Null),
                                    "profile_id": existing_obj.get("profile_id").cloned().unwrap_or(Value::Null),
                                    "data_root": existing_obj.get("data_root").cloned().unwrap_or(Value::Null),
                                }
                            });
                            return Err(DoudouError::new(
                                "PROFILE_IN_USE",
                                format!(
                                    "Profile {} is already owned by a broker.",
                                    self.profile_id
                                ),
                            )
                            .with_details(details));
                        }
                    }
                    // Stale or corrupt lock file from a dead process — remove and retry.
                    let _ = fs::remove_file(&self.path);
                }
                Err(err) => {
                    return Err(DoudouError::new(
                        "IO_ERROR",
                        format!("Failed creating lock file {}: {err}", self.path.display()),
                    ));
                }
            }
        }

        Err(DoudouError::new(
            "PROFILE_IN_USE",
            format!("Profile {} is already owned by a broker.", self.profile_id),
        ))
    }

    /// Release the lock file if this instance still owns it.
    pub fn release(&mut self) {
        let Some(owner) = self.owner.take() else {
            return;
        };
        if let Ok(Some(current)) = read_json(&self.path, true)
            && let Some(obj) = current.as_object()
        {
            let cur_pid = obj
                .get("pid")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            let cur_start = obj
                .get("process_start")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            if cur_pid == Some(owner.pid) && cur_start == owner.process_start {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}

impl Drop for ProfileLock {
    fn drop(&mut self) {
        self.release();
    }
}

/// Return `true` when `path` contains a valid lock owned by a currently live OS process.
pub fn lock_is_live(path: &Path) -> bool {
    let Ok(Some(owner)) = read_json(path, true) else {
        return false;
    };
    let Some(obj) = owner.as_object() else {
        return false;
    };
    let pid = obj
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    if pid == 0 {
        return false;
    }
    let start = obj
        .get("process_start")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let expected = ProcessIdentity::new(pid, start, None);
    let actual = process_identity(pid);
    same_process(Some(&expected), actual.as_ref())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn profile_lock_acquires_rejects_live_duplicate_and_reclaims_stale() {
        let dir = TempDir::new().expect("tempdir");
        let lock_path = dir.path().join(".lock.json");

        let mut first = ProfileLock::new(
            lock_path.clone(),
            "work",
            dir.path().to_path_buf(),
            "broker",
        );
        let owner = first.acquire().expect("first acquire");
        assert_eq!(owner.schema, PROFILE_LOCK_SCHEMA);
        assert!(lock_is_live(&lock_path));

        // Second acquire while live must fail with PROFILE_IN_USE.
        let mut second = ProfileLock::new(
            lock_path.clone(),
            "work",
            dir.path().to_path_buf(),
            "broker",
        );
        let err = second.acquire().expect_err("duplicate lock must fail");
        assert_eq!(err.code, "PROFILE_IN_USE");

        // Releasing cleans up the file.
        first.release();
        assert!(!lock_path.exists());
        assert!(!lock_is_live(&lock_path));

        // Stale lock with a dead PID is reclaimed automatically.
        fs::write(
            &lock_path,
            r#"{"schema":"doudou.profile-lock.v2","profile_id":"work","data_root":".","pid":99999999,"process_start":"2020-01-01T00:00:00Z","command":"broker","created_at":"2020-01-01T00:00:00Z"}"#,
        )
        .expect("write stale lock");
        assert!(!lock_is_live(&lock_path));
        let reclaimed = second.acquire().expect("reclaim stale lock");
        assert_eq!(reclaimed.pid, std::process::id());
        second.release();
    }
}
