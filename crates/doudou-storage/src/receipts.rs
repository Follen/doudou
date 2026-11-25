//! Persistent send and interaction receipts stored under `sends/` and `interactions/<domain>/`.

use std::path::Path;

use chrono::Utc;
use doudou_core::{
    Result, atomic_write_json, profile_paths, random_id, read_json, safe_segment, utc_now,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Destination bucket for write receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptKind {
    /// Chat send/reply/batch receipt under `message/profiles/<profile>/sends/<YYYY-MM-DD>/...`.
    Send,
    /// Interaction receipt under `message/profiles/<profile>/interactions/<domain>/<YYYY-MM-DD>/...`.
    Interaction,
}

fn relative_display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Atomically persist `receipt.json` and `verification.json` for a completed external write.
pub fn write_receipt(
    home: &Path,
    data_root: &Path,
    profile: &str,
    domain: &str,
    operation: &str,
    record: &Value,
    kind: ReceiptKind,
) -> Result<Value> {
    let paths = profile_paths(home, data_root, profile)?;
    let now = Utc::now();
    let identifier = random_id("receipt");
    let base = match kind {
        ReceiptKind::Send => paths.sends,
        ReceiptKind::Interaction => paths.interactions.join(safe_segment(domain, 64)?),
    };
    let day = now.format("%Y-%m-%d").to_string();
    let stamp = now.format("%H%M%S%6f").to_string();
    let directory = base.join(day).join(format!("{stamp}__{identifier}"));

    let mut receipt = Map::new();
    receipt.insert("schema".to_owned(), json!("doudou.write-receipt.v1"));
    receipt.insert("receipt_id".to_owned(), json!(identifier));
    receipt.insert("profile_id".to_owned(), json!(paths.profile_id));
    receipt.insert("domain".to_owned(), json!(domain));
    receipt.insert("operation".to_owned(), json!(operation));
    receipt.insert("completed_at".to_owned(), json!(utc_now()));
    if let Some(record_obj) = record.as_object() {
        for (k, v) in record_obj {
            receipt.insert(k.clone(), v.clone());
        }
    }

    let verification = receipt
        .get("verification")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!({}));

    let receipt_path = directory.join("receipt.json");
    let verification_path = directory.join("verification.json");
    atomic_write_json(&receipt_path, &Value::Object(receipt), None)?;
    atomic_write_json(&verification_path, &verification, None)?;

    Ok(json!({
        "directory": relative_display(&directory, data_root),
        "receipt": relative_display(&receipt_path, data_root),
        "verification": relative_display(&verification_path, data_root),
    }))
}

/// Read a receipt JSON file from disk if present.
pub fn read_receipt(path: &Path) -> Result<Option<Value>> {
    read_json(path, true)
}
