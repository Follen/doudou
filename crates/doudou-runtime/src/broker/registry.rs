//! Global broker registry under `<doudou_home>/brokers/` and broker lifecycle management.

use std::fs;
use std::path::{Path, PathBuf};

use doudou_core::{
    DoudouError, ProfilePaths, Result, absolutize_path, atomic_write_json, broker_registry_root,
    profile_paths, read_json, sha256_str,
};
use serde_json::{Value, json};

use crate::broker::client::BrokerClient;
use crate::broker::protocol::{
    BROKER_DESCRIPTOR_SCHEMA, BROKER_REGISTRATION_SCHEMA, BrokerDescriptor, BrokerRegistration,
    CommandSemantics,
};
use crate::process::{ProcessIdentity, process_identity, same_process};

/// Compute the deterministic registration file path under `<home>/brokers/` for `(data_root, profile_id)`.
pub fn registration_path(home: &Path, data_root: &Path, profile_id: &str) -> PathBuf {
    let resolved_root = absolutize_path(data_root);
    let identity = format!("{}\0{profile_id}", resolved_root.display());
    broker_registry_root(home).join(format!("{}.json", sha256_str(&identity)))
}

/// Check whether a persisted broker descriptor belongs to `(data_root, profile_id)` and is live.
pub fn descriptor_is_live(descriptor: &Value, data_root: &Path, profile_id: &str) -> bool {
    let Some(obj) = descriptor.as_object() else {
        return false;
    };
    if obj.get("schema").and_then(Value::as_str) != Some(BROKER_DESCRIPTOR_SCHEMA) {
        return false;
    }
    let Some(desc_root) = obj.get("data_root").and_then(Value::as_str) else {
        return false;
    };
    if absolutize_path(Path::new(desc_root)) != absolutize_path(data_root) {
        return false;
    }
    if obj.get("profile_id").and_then(Value::as_str) != Some(profile_id) {
        return false;
    }
    let pid = obj
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    if pid == 0 {
        return false;
    }
    let expected = ProcessIdentity::new(
        pid,
        obj.get("process_start")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        None,
    );
    let actual = process_identity(pid);
    same_process(Some(&expected), actual.as_ref())
}

/// Write a broker registration record under `<home>/brokers/` and return its file path.
pub fn register_broker(paths: &ProfilePaths, descriptor: &BrokerDescriptor) -> Result<PathBuf> {
    let reg_path = registration_path(&paths.home, &paths.data_root, &paths.profile_id);
    let executable = std::env::current_exe()
        .map(|p| absolutize_path(&p).display().to_string())
        .unwrap_or_default();
    let record = BrokerRegistration {
        schema: BROKER_REGISTRATION_SCHEMA.to_owned(),
        pid: descriptor.pid,
        process_start: descriptor.process_start.clone(),
        profile_id: paths.profile_id.clone(),
        data_root: absolutize_path(&paths.data_root).display().to_string(),
        executable,
        descriptor: absolutize_path(&paths.broker_descriptor)
            .display()
            .to_string(),
        token: absolutize_path(&paths.broker_token).display().to_string(),
    };
    atomic_write_json(&reg_path, &record, Some(0o600))?;
    Ok(reg_path)
}

/// Remove a previously written broker registration file (best-effort).
pub fn unregister_broker(registration: Option<&Path>) {
    if let Some(path) = registration {
        let _ = fs::remove_file(path);
    }
}

/// List all live brokers registered under `<home>/brokers/`, pruning stale dead entries.
pub fn list_brokers(home: &Path) -> Result<Vec<Value>> {
    let root = broker_registry_root(home);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&root).map_err(|err| {
        DoudouError::new(
            "IO_ERROR",
            format!("Failed reading broker registry {}: {err}", root.display()),
        )
    })?;

    let mut active = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(Some(raw)) = read_json(&path, true) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        let Ok(reg) = serde_json::from_value::<BrokerRegistration>(raw) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        let expected = ProcessIdentity::new(reg.pid, reg.process_start.clone(), None);
        let actual = process_identity(reg.pid);
        if !same_process(Some(&expected), actual.as_ref()) {
            let _ = fs::remove_file(&path);
            continue;
        }
        let descriptor = read_json(Path::new(&reg.descriptor), true).ok().flatten();
        active.push(json!({
            "registration_file": path.display().to_string(),
            "pid": reg.pid,
            "process_start": reg.process_start,
            "profile_id": reg.profile_id,
            "data_root": reg.data_root,
            "executable": reg.executable,
            "descriptor": descriptor,
            "live": true,
        }));
    }
    active.sort_by(|left, right| {
        let key = |v: &Value| {
            (
                v.get("profile_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                v.get("pid").and_then(Value::as_u64).unwrap_or(0),
            )
        };
        key(left).cmp(&key(right))
    });
    Ok(active)
}

/// Query status for a single profile's broker, contacting it over TCP if running.
pub async fn broker_status(home: &Path, data_root: &Path, profile_name: &str) -> Result<Value> {
    let paths = profile_paths(home, data_root, profile_name)?;
    let descriptor = read_json(&paths.broker_descriptor, true)?;
    let Some(desc_val) = descriptor else {
        return Ok(json!({
            "running": false,
            "profile_id": paths.profile_id,
            "data_root": absolutize_path(data_root).display().to_string(),
        }));
    };
    if !descriptor_is_live(&desc_val, data_root, &paths.profile_id) {
        let _ = fs::remove_file(&paths.broker_descriptor);
        let _ = fs::remove_file(&paths.broker_token);
        return Ok(json!({
            "running": false,
            "stale_cleaned": true,
            "profile_id": paths.profile_id,
            "data_root": absolutize_path(data_root).display().to_string(),
        }));
    }
    let client = BrokerClient::new(home, data_root, profile_name)?;
    match client
        .request_async("status", json!({}), CommandSemantics::Read, None, false)
        .await
    {
        Ok(resp) => Ok(json!({
            "running": true,
            "profile_id": paths.profile_id,
            "descriptor": desc_val,
            "status": resp.get("result").cloned().unwrap_or(Value::Null),
        })),
        Err(err) => Ok(json!({
            "running": false,
            "profile_id": paths.profile_id,
            "descriptor": desc_val,
            "error_code": err.code,
        })),
    }
}

/// Gracefully stop a running broker for `(data_root, profile_name)` via its `shutdown` control RPC.
pub async fn stop_broker(home: &Path, data_root: &Path, profile_name: &str) -> Result<Value> {
    let paths = profile_paths(home, data_root, profile_name)?;
    let descriptor = read_json(&paths.broker_descriptor, true)?;
    let Some(desc_val) = descriptor else {
        return Ok(json!({
            "stopped": true,
            "was_running": false,
            "profile_id": paths.profile_id,
        }));
    };
    if !descriptor_is_live(&desc_val, data_root, &paths.profile_id) {
        let _ = fs::remove_file(&paths.broker_descriptor);
        let _ = fs::remove_file(&paths.broker_token);
        unregister_broker(Some(&registration_path(home, data_root, &paths.profile_id)));
        return Ok(json!({
            "stopped": true,
            "was_running": false,
            "stale_cleaned": true,
            "profile_id": paths.profile_id,
        }));
    }

    let client = BrokerClient::new(home, data_root, profile_name)?;
    let _ = client
        .request_async(
            "shutdown",
            json!({}),
            CommandSemantics::Control,
            None,
            false,
        )
        .await;
    let stopped = client
        .wait_stopped(std::time::Duration::from_secs(10))
        .await;
    Ok(json!({
        "stopped": stopped,
        "was_running": true,
        "profile_id": paths.profile_id,
    }))
}

/// Gracefully stop every live broker registered under `<home>/brokers/`.
pub async fn stop_all_brokers(home: &Path) -> Result<Value> {
    let brokers = list_brokers(home)?;
    let mut results = Vec::with_capacity(brokers.len());
    for entry in &brokers {
        let Some(profile_id) = entry.get("profile_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(data_root) = entry.get("data_root").and_then(Value::as_str) else {
            continue;
        };
        let outcome = stop_broker(home, Path::new(data_root), profile_id).await?;
        results.push(outcome);
    }
    Ok(json!({
        "stopped_count": results.len(),
        "results": results,
    }))
}
