//! Single-use write plans enforcing strict `ConfirmationRequired -> Executing -> Terminal` state transitions.

use std::fmt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use doudou_core::{
    DoudouError, Result, atomic_write_json, profile_paths, random_id, read_json, sha256_str,
    utc_now,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Default lifetime in seconds before an unconfirmed write plan expires.
pub const DEFAULT_PLAN_TTL_SECONDS: u64 = 900;

/// Schema version for persisted write-plan documents.
pub const WRITE_PLAN_SCHEMA: &str = "doudou.write-plan.v1";

/// Strict single-use lifecycle state of a write plan.
///
/// Valid transitions:
/// - `ConfirmationRequired -> Executing` (via [`confirm_plan`], strictly once)
/// - `Executing -> Sent | Failed | Unknown` (via [`complete_plan`])
///
/// Rejecting `Executing` (as well as all terminal states) in [`confirm_plan`] guarantees at-most-once
/// external write execution even across broker crashes and retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Plan was previewed and awaits explicit user confirmation.
    ConfirmationRequired,
    /// Plan was confirmed and claimed for one external write execution.
    Executing,
    /// External write completed and verified.
    Sent,
    /// External write failed before submission.
    Failed,
    /// Outcome could not be verified (e.g. crash, timeout, or cancellation during dispatch).
    Unknown,
}

impl PlanStatus {
    /// Lowercase `snake_case` string label matching the JSON schema.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfirmationRequired => "confirmation_required",
            Self::Executing => "executing",
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }

    /// Return `true` when this status is terminal (`Sent`, `Failed`, or `Unknown`).
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Sent | Self::Failed | Self::Unknown)
    }
}

impl fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Terminal outcome passed to [`complete_plan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalPlanStatus {
    Sent,
    Failed,
    Unknown,
}

impl From<TerminalPlanStatus> for PlanStatus {
    fn from(value: TerminalPlanStatus) -> Self {
        match value {
            TerminalPlanStatus::Sent => Self::Sent,
            TerminalPlanStatus::Failed => Self::Failed,
            TerminalPlanStatus::Unknown => Self::Unknown,
        }
    }
}

/// Persisted write-plan record stored under `message/profiles/<profile>/plans/<plan_id>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WritePlan {
    pub schema: String,
    pub plan_id: String,
    pub profile_id: String,
    pub operation: String,
    pub payload: Value,
    pub payload_sha256: String,
    pub created_at: String,
    pub expires_at: String,
    pub status: PlanStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<String>,
}

/// Compute a canonical SHA-256 hash over `{operation, payload}` with recursively sorted object keys.
pub fn payload_hash(operation: &str, payload: &Value) -> String {
    let envelope = json!({
        "operation": operation,
        "payload": canonicalize_value(payload),
    });
    let serialized = serde_json::to_string(&envelope).unwrap_or_default();
    sha256_str(&serialized)
}

fn canonicalize_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_value).collect()),
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by_key(|(left, _)| *left);
            let mut sorted = serde_json::Map::with_capacity(entries.len());
            for (k, v) in entries {
                sorted.insert(k.clone(), canonicalize_value(v));
            }
            Value::Object(sorted)
        }
        other => other.clone(),
    }
}

fn plan_file_path(home: &Path, data_root: &Path, profile: &str, plan_id: &str) -> Result<PathBuf> {
    if plan_id.is_empty()
        || matches!(plan_id, "." | "..")
        || Path::new(plan_id).file_name().and_then(|n| n.to_str()) != Some(plan_id)
    {
        return Err(
            DoudouError::new("PLAN_NOT_FOUND", "The write plan does not exist.")
                .with_details(json!({ "plan_id": plan_id })),
        );
    }
    Ok(profile_paths(home, data_root, profile)?
        .plans
        .join(format!("{plan_id}.json")))
}

fn parse_plan_document(raw: &Value, plan_id: &str) -> Result<WritePlan> {
    if let Ok(plan) = serde_json::from_value::<WritePlan>(raw.clone()) {
        return Ok(plan);
    }
    // Fail closed if an older on-disk plan carries a legacy status such as `"confirmed"`.
    if let Some(status) = raw.get("status").and_then(Value::as_str)
        && status != PlanStatus::ConfirmationRequired.as_str()
    {
        return Err(DoudouError::new(
            "PLAN_ALREADY_USED",
            "The write plan is no longer executable.",
        )
        .with_details(json!({ "plan_id": plan_id, "status": status })));
    }
    Err(
        DoudouError::new("INVALID_PLAN", "The write plan document is malformed.")
            .with_details(json!({ "plan_id": plan_id })),
    )
}

/// Create and persist a new unconfirmed write plan in [`PlanStatus::ConfirmationRequired`].
pub fn create_plan(
    home: &Path,
    data_root: &Path,
    profile: &str,
    operation: &str,
    payload: Value,
    ttl_seconds: u64,
) -> Result<WritePlan> {
    let paths = profile_paths(home, data_root, profile)?;
    let created = Utc::now();
    let ttl = i64::try_from(ttl_seconds).unwrap_or(i64::MAX);
    let expires = created
        .checked_add_signed(Duration::seconds(ttl))
        .unwrap_or(created);
    let plan = WritePlan {
        schema: WRITE_PLAN_SCHEMA.to_owned(),
        plan_id: random_id("plan"),
        profile_id: paths.profile_id,
        operation: operation.to_owned(),
        payload_sha256: payload_hash(operation, &payload),
        payload,
        created_at: created.to_rfc3339_opts(SecondsFormat::Millis, true),
        expires_at: expires.to_rfc3339_opts(SecondsFormat::Millis, true),
        status: PlanStatus::ConfirmationRequired,
        confirmed_at: None,
        completed_at: None,
        receipt: None,
    };
    let path = paths.plans.join(format!("{}.json", plan.plan_id));
    atomic_write_json(&path, &plan, None)?;
    Ok(plan)
}

/// Atomically confirm a write plan by transitioning `ConfirmationRequired -> Executing`.
///
/// Rejects argument drift (`PLAN_DRIFTED`), expired plans (`PLAN_EXPIRED`), and any plan whose
/// status is not [`PlanStatus::ConfirmationRequired`] (`PLAN_ALREADY_USED`) — including
/// [`PlanStatus::Executing`] after a crash.
pub fn confirm_plan(
    home: &Path,
    data_root: &Path,
    profile: &str,
    plan_id: &str,
    operation: &str,
    payload: &Value,
) -> Result<WritePlan> {
    let path = plan_file_path(home, data_root, profile, plan_id)?;
    let Some(raw) = read_json(&path, true)? else {
        return Err(
            DoudouError::new("PLAN_NOT_FOUND", "The write plan does not exist.")
                .with_details(json!({ "plan_id": plan_id })),
        );
    };
    if !raw.is_object() {
        return Err(
            DoudouError::new("PLAN_NOT_FOUND", "The write plan does not exist.")
                .with_details(json!({ "plan_id": plan_id })),
        );
    }
    let mut plan = parse_plan_document(&raw, plan_id)?;
    if plan.operation != operation || plan.payload_sha256 != payload_hash(operation, payload) {
        return Err(DoudouError::new(
            "PLAN_DRIFTED",
            "The confirmed write arguments differ from the preview.",
        )
        .with_details(json!({ "plan_id": plan_id })));
    }
    let expires = DateTime::parse_from_rfc3339(&plan.expires_at).map_err(|_| {
        DoudouError::new(
            "INVALID_PLAN",
            "The write plan has an invalid expires_at timestamp.",
        )
        .with_details(json!({ "plan_id": plan_id }))
    })?;
    if expires.with_timezone(&Utc) <= Utc::now() {
        return Err(DoudouError::new(
            "PLAN_EXPIRED",
            "The write plan expired before confirmation.",
        )
        .with_details(json!({ "plan_id": plan_id })));
    }
    if plan.status != PlanStatus::ConfirmationRequired {
        return Err(DoudouError::new(
            "PLAN_ALREADY_USED",
            "The write plan is no longer executable.",
        )
        .with_details(json!({
            "plan_id": plan_id,
            "status": plan.status,
        })));
    }
    plan.status = PlanStatus::Executing;
    plan.confirmed_at = Some(utc_now());
    atomic_write_json(&path, &plan, None)?;
    Ok(plan)
}

/// Load a write plan and optionally assert that it belongs to `expected_operation`.
pub fn load_plan(
    home: &Path,
    data_root: &Path,
    profile: &str,
    plan_id: &str,
    expected_operation: Option<&str>,
) -> Result<WritePlan> {
    let path = plan_file_path(home, data_root, profile, plan_id)?;
    let Some(raw) = read_json(&path, true)? else {
        return Err(
            DoudouError::new("PLAN_NOT_FOUND", "The write plan does not exist.")
                .with_details(json!({ "plan_id": plan_id })),
        );
    };
    let plan = parse_plan_document(&raw, plan_id)?;
    if let Some(expected) = expected_operation
        && plan.operation != expected
    {
        return Err(DoudouError::new(
            "PLAN_OPERATION_MISMATCH",
            "The plan belongs to a different operation.",
        )
        .with_details(json!({
            "plan_id": plan_id,
            "expected": expected,
            "actual": plan.operation,
        })));
    }
    Ok(plan)
}

/// Finalize an [`PlanStatus::Executing`] plan into a terminal state (`Sent`, `Failed`, or `Unknown`).
pub fn complete_plan(
    home: &Path,
    data_root: &Path,
    profile: &str,
    plan_id: &str,
    status: TerminalPlanStatus,
    receipt: Option<&str>,
) -> Result<WritePlan> {
    let path = plan_file_path(home, data_root, profile, plan_id)?;
    let Some(raw) = read_json(&path, true)? else {
        return Err(
            DoudouError::new("PLAN_NOT_FOUND", "The write plan does not exist.")
                .with_details(json!({ "plan_id": plan_id })),
        );
    };
    let mut plan = parse_plan_document(&raw, plan_id)?;
    if plan.status != PlanStatus::Executing {
        return Err(DoudouError::new(
            "INVALID_PLAN_TRANSITION",
            "Only an executing write plan can be completed.",
        )
        .with_details(json!({
            "plan_id": plan_id,
            "from": plan.status,
            "to": PlanStatus::from(status),
        })));
    }
    plan.status = status.into();
    plan.completed_at = Some(utc_now());
    plan.receipt = receipt.map(ToOwned::to_owned);
    atomic_write_json(&path, &plan, None)?;
    Ok(plan)
}
