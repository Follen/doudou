//! Structured error type shared across Doudou crates.

use std::fmt;

use serde_json::{Map, Value, json};

/// Application error with a stable machine-readable code and optional details.
#[derive(Debug, Clone)]
pub struct DoudouError {
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
    pub exit_code: i32,
}

impl DoudouError {
    /// Create a new error with default exit code `1`.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
            exit_code: 1,
        }
    }

    /// Attach structured JSON details to this error.
    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Override the CLI process exit code.
    #[must_use]
    pub fn with_exit_code(mut self, exit_code: i32) -> Self {
        self.exit_code = exit_code;
        self
    }

    /// Mark this error's JSON `details` with `"write_started": true`, merging into an existing
    /// JSON object or creating one when absent.
    #[must_use]
    pub fn mark_write_started(mut self) -> Self {
        match self.details {
            Some(Value::Object(ref mut map)) => {
                map.insert("write_started".to_owned(), json!(true));
            }
            Some(other) => {
                let mut map = Map::new();
                map.insert("write_started".to_owned(), json!(true));
                map.insert("cause_details".to_owned(), other);
                self.details = Some(Value::Object(map));
            }
            None => {
                self.details = Some(json!({ "write_started": true }));
            }
        }
        self
    }

    /// Return `true` when `details["write_started"]` is explicitly `true`.
    pub fn is_write_started(&self) -> bool {
        self.details
            .as_ref()
            .and_then(|d| d.get("write_started"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// Format this error as an NDJSON `error` event object.
    pub fn as_event(&self) -> Value {
        let mut map = Map::new();
        map.insert("event".to_owned(), json!("error"));
        map.insert("code".to_owned(), json!(self.code));
        map.insert("message".to_owned(), json!(self.message));
        if let Some(details) = &self.details {
            map.insert("details".to_owned(), details.clone());
        }
        Value::Object(map)
    }
}

impl fmt::Display for DoudouError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for DoudouError {}

/// Convenience result alias for fallible Doudou operations.
pub type Result<T> = std::result::Result<T, DoudouError>;
