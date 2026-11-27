//! Wire protocol types, frame limits, and bounded line reader for loopback TCP broker RPC.

use doudou_core::{DoudouError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt as _};

/// Maximum JSON-line frame size in bytes (16 MiB) enforced on both broker server and client.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Default total per-command execution timeout in seconds.
pub const DEFAULT_COMMAND_TIMEOUT_SECONDS: f64 = 90.0;

/// Maximum permitted per-command execution timeout in seconds.
pub const MAX_COMMAND_TIMEOUT_SECONDS: f64 = 900.0;

/// Minimum permitted per-command execution timeout in seconds.
pub const MIN_COMMAND_TIMEOUT_SECONDS: f64 = 0.1;

/// Schema version for per-profile `runtime/broker.json` descriptor files.
pub const BROKER_DESCRIPTOR_SCHEMA: &str = "doudou.broker-descriptor.v1";

/// Schema version for user-level `<doudou_home>/brokers/<hash>.json` registrations.
pub const BROKER_REGISTRATION_SCHEMA: &str = "doudou.broker-registration.v1";

/// Deterministic pre-submission write errors that are preserved verbatim instead of becoming
/// `WRITE_RESULT_UNKNOWN`.
pub const DETERMINISTIC_WRITE_ERRORS: &[&str] = &[
    "PAGE_ADAPTER_DRIFTED",
    "PROTOCOL_SCHEMA_DRIFTED",
    "PLAN_DRIFTED",
    "PLAN_EXPIRED",
    "PLAN_NOT_FOUND",
    "PLAN_ALREADY_USED",
    "PLAN_OPERATION_MISMATCH",
    "INVALID_PLAN_TRANSITION",
    "IMAGE_NOT_FOUND",
    "COMPOSER_PLAN_MISMATCH",
    "CONVERSATION_UI_AMBIGUOUS",
    "REPLY_TARGET_NOT_VISIBLE",
    "MENTION_TARGET_NOT_VISIBLE",
    "MENTION_UNSUPPORTED",
    "STRANGER_WRITE_UNSUPPORTED",
    "MESSAGE_TARGET_NOT_VISIBLE",
    "COMMENT_TARGET_NOT_VISIBLE",
];

/// Command execution semantics governing retry, timeout, and verification recovery policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandSemantics {
    /// Read-only operation; eligible for interactive verification recovery and safe reconnects.
    Read,
    /// External write operation; strictly at-most-once, never automatically replayed.
    Write,
    /// Broker control command (e.g. `shutdown`, `session.refresh`).
    Control,
}

impl CommandSemantics {
    /// Return the canonical wire label (`"read"`, `"write"`, or `"control"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Control => "control",
        }
    }

    /// Parse a semantics string strictly.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "control" => Some(Self::Control),
            _ => None,
        }
    }
}

/// Loopback TCP endpoint descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerEndpoint {
    /// Transport protocol (`"tcp"`).
    pub transport: String,
    /// Loopback host (`"127.0.0.1"`).
    pub host: String,
    /// Bound ephemeral TCP port.
    pub port: u16,
}

/// Persisted descriptor written to `profiles/<profile>/runtime/broker.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerDescriptor {
    /// Descriptor schema version (`doudou.broker-descriptor.v1`).
    pub schema: String,
    /// Broker OS process identifier.
    pub pid: u32,
    /// Broker process creation timestamp.
    pub process_start: Option<String>,
    /// Resolved project data root path.
    pub data_root: String,
    /// Profile identifier.
    pub profile_id: String,
    /// Loopback TCP endpoint.
    pub endpoint: BrokerEndpoint,
    /// Relative filename of the per-broker auth token file (`"broker.token"`).
    pub token_file: String,
    /// Current browser session mode (`"headless"`, `"headed_takeover"`, `"fake"`).
    pub mode: String,
    /// Optional browser binary version string.
    pub browser_version: Option<String>,
    /// UTC timestamp when the broker started.
    pub created_at: String,
    /// UTC timestamp of the most recent completed command.
    pub last_activity_at: String,
    /// Current interactive takeover snapshot, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub takeover: Option<Value>,
}

/// Global broker registry entry stored under `<doudou_home>/brokers/<hash>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerRegistration {
    /// Registration schema version (`doudou.broker-registration.v1`).
    pub schema: String,
    /// Broker OS process identifier.
    pub pid: u32,
    /// Broker process creation timestamp.
    pub process_start: Option<String>,
    /// Profile identifier.
    pub profile_id: String,
    /// Project data root path.
    pub data_root: String,
    /// Executable path of the broker process.
    pub executable: String,
    /// Absolute path to `broker.json`.
    pub descriptor: String,
    /// Absolute path to `broker.token`.
    pub token: String,
}

/// JSON-line request envelope sent from [`super::BrokerClient`] to [`super::BrokerServer`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrokerRequest {
    /// Per-broker shared secret token.
    pub token: String,
    /// Caller-assigned idempotency identifier.
    pub request_id: String,
    /// Command name (e.g. `"ping"`, `"chat.send.execute"`).
    pub command: String,
    /// Command semantics (`read`, `write`, or `control`).
    pub semantics: String,
    /// Command parameters JSON object.
    #[serde(default)]
    pub params: Value,
    /// Optional total command timeout override in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_timeout_seconds: Option<f64>,
}

/// Compute the bounded total command timeout in seconds matching Python's `command_timeout_seconds`.
pub fn command_timeout_seconds(command: &str, params: &Value) -> f64 {
    match command {
        "page.doctor" => 180.0,
        "videos.collect" => 600.0,
        "video.collect" => 900.0,
        "login.wait" | "protocol.capture" | "session.auth.wait" => {
            let requested = params
                .get("timeout_seconds")
                .and_then(Value::as_f64)
                .unwrap_or(600.0);
            (requested + 30.0).clamp(30.0, MAX_COMMAND_TIMEOUT_SECONDS)
        }
        _ => DEFAULT_COMMAND_TIMEOUT_SECONDS,
    }
}

/// Constant-time string comparison for broker authentication tokens.
pub fn constant_time_eq(left: &str, right: &str) -> bool {
    let a = left.as_bytes();
    let b = right.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Read a single `\n`-delimited frame from `reader` without ever buffering more than `max_bytes`.
///
/// Returns `Ok(None)` on clean EOF before any bytes are read.
/// Returns `Err(BROKER_FRAME_TOO_LARGE)` as soon as a line exceeds `max_bytes`.
pub async fn read_bounded_line<R>(reader: &mut R, max_bytes: usize) -> Result<Option<Vec<u8>>>
where
    R: AsyncBufRead + Unpin,
{
    let mut buf = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(|err| {
            DoudouError::new(
                "BROKER_IO_ERROR",
                format!("Failed reading broker frame: {err}"),
            )
        })?;
        if available.is_empty() {
            if buf.is_empty() {
                return Ok(None);
            }
            return Ok(Some(buf));
        }

        if let Some(newline_pos) = available.iter().position(|&b| b == b'\n') {
            let take = newline_pos + 1;
            if buf.len().saturating_add(take) > max_bytes {
                reader.consume(take);
                return Err(DoudouError::new(
                    "BROKER_FRAME_TOO_LARGE",
                    "The broker frame exceeded the maximum permitted byte limit.",
                )
                .with_details(json!({ "limit_bytes": max_bytes })));
            }
            buf.extend_from_slice(&available[..take]);
            reader.consume(take);
            return Ok(Some(buf));
        }

        let chunk_len = available.len();
        if buf.len().saturating_add(chunk_len) > max_bytes {
            reader.consume(chunk_len);
            return Err(DoudouError::new(
                "BROKER_FRAME_TOO_LARGE",
                "The broker frame exceeded the maximum permitted byte limit.",
            )
            .with_details(json!({ "limit_bytes": max_bytes })));
        }
        buf.extend_from_slice(available);
        reader.consume(chunk_len);
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::BufReader;

    use super::*;

    #[test]
    fn command_timeouts_match_contract() {
        assert!(
            (command_timeout_seconds("videos.collect", &json!({})) - 600.0).abs() < f64::EPSILON
        );
        assert!(
            (command_timeout_seconds("video.collect", &json!({})) - 900.0).abs() < f64::EPSILON
        );
        assert!(
            (command_timeout_seconds("messages.collect", &json!({})) - 90.0).abs() < f64::EPSILON
        );
        assert!((command_timeout_seconds("page.doctor", &json!({})) - 180.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn read_bounded_line_enforces_exact_limit() {
        let mut exact = BufReader::new(&b"1234\n"[..]);
        let line = read_bounded_line(&mut exact, 5)
            .await
            .expect("exact limit should succeed")
            .expect("non-empty line");
        assert_eq!(line, b"1234\n");

        let mut too_large = BufReader::new(&b"12345\n"[..]);
        let err = read_bounded_line(&mut too_large, 5)
            .await
            .expect_err("oversized line must fail");
        assert_eq!(err.code, "BROKER_FRAME_TOO_LARGE");
        assert_eq!(err.details.expect("details")["limit_bytes"], 5);
    }
}
