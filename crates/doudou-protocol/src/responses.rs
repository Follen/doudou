//! Page-context HTTP JSON response classification without leaking response bodies.

use doudou_core::{DoudouError, Result, sha256_str};
use serde_json::{Map, Value, json};

/// Operation-specific error codes for empty, non-JSON, and business-error responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseCodes {
    pub empty: &'static str,
    pub invalid: &'static str,
    pub failed: &'static str,
}

/// Raw HTTP response captured from a browser `fetch` call.
#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub http_status: u16,
    pub content_type: String,
    pub retry_after: Option<String>,
    pub text: String,
}

/// Build sanitized diagnostic metadata (status, content-type, byte count, SHA-256).
pub fn response_metadata(result: &FetchResponse) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("http_status".to_owned(), json!(result.http_status));
    map.insert("content_type".to_owned(), json!(result.content_type));
    map.insert("retry_after".to_owned(), json!(result.retry_after));
    map.insert("response_bytes".to_owned(), json!(result.text.len()));
    map.insert(
        "response_sha256".to_owned(),
        json!(sha256_str(&result.text)),
    );
    map
}

fn has_verification_kick(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().any(has_verification_kick),
        Value::Object(map) => {
            if map
                .get("decision")
                .and_then(Value::as_str)
                .is_some_and(|s| s.eq_ignore_ascii_case("KICK"))
            {
                return true;
            }
            map.values().any(has_verification_kick)
        }
        _ => false,
    }
}

fn is_verification_document(text: &str, content_type: &str) -> bool {
    const MARKERS: &[&str] = &[
        "captcha",
        "verify",
        "verification",
        "second-verify",
        "验证码",
        "安全验证",
        "滑块",
    ];
    if !content_type.to_lowercase().contains("html") && !text.trim_start().starts_with('<') {
        return false;
    }
    let sample_end = text
        .char_indices()
        .nth(64_000)
        .map_or(text.len(), |(idx, _)| idx);
    let sample = text[..sample_end].to_lowercase();
    MARKERS.iter().any(|m| sample.contains(m))
}

/// Classify a page-context JSON response into a parsed JSON object or a stable `DoudouError`.
pub fn parse_json_response(
    result: &FetchResponse,
    codes: ResponseCodes,
    authenticated: bool,
    verification_visible: bool,
    operation: &str,
) -> Result<Map<String, Value>> {
    let mut metadata = response_metadata(result);
    metadata.insert("operation".to_owned(), json!(operation));

    if !authenticated {
        return Err(
            DoudouError::new("LOGIN_REQUIRED", "The Profile login session is missing.")
                .with_details(Value::Object(metadata)),
        );
    }
    if result.http_status == 429 {
        return Err(
            DoudouError::new("RATE_LIMITED", "The upstream request was rate limited.")
                .with_details(Value::Object(metadata)),
        );
    }
    if (500..=599).contains(&result.http_status) {
        return Err(DoudouError::new(
            "UPSTREAM_UNAVAILABLE",
            "The upstream service is temporarily unavailable.",
        )
        .with_details(Value::Object(metadata)));
    }
    if result.text.is_empty() {
        return Err(
            DoudouError::new(codes.empty, "The protocol response was empty.")
                .with_details(Value::Object(metadata)),
        );
    }

    let Ok(parsed) = serde_json::from_str::<Value>(&result.text) else {
        if verification_visible || is_verification_document(&result.text, &result.content_type) {
            return Err(DoudouError::new(
                "VERIFICATION_REQUIRED",
                "The protocol request reached account verification.",
            )
            .with_details(Value::Object(metadata)));
        }
        return Err(
            DoudouError::new(codes.invalid, "The protocol response was not valid JSON.")
                .with_details(Value::Object(metadata)),
        );
    };

    let gateway_kick = has_verification_kick(&parsed);
    if verification_visible || gateway_kick {
        if gateway_kick {
            metadata.insert("gateway_decision".to_owned(), json!("KICK"));
        }
        return Err(DoudouError::new(
            "VERIFICATION_REQUIRED",
            "The protocol request reached account verification.",
        )
        .with_details(Value::Object(metadata)));
    }

    let Value::Object(obj) = parsed else {
        return Err(DoudouError::new(
            codes.invalid,
            "The protocol response JSON was not an object.",
        )
        .with_details(Value::Object(metadata)));
    };

    let status_code = obj.get("status_code").or_else(|| obj.get("statusCode"));
    let business_ok = match status_code {
        None | Some(Value::Null) => true,
        Some(Value::Number(n)) => n.as_i64() == Some(0),
        _ => false,
    };
    if !(200..300).contains(&result.http_status) || !business_ok {
        metadata.insert(
            "status_code".to_owned(),
            status_code.cloned().unwrap_or(Value::Null),
        );
        return Err(DoudouError::new(
            codes.failed,
            "The protocol request returned a business error.",
        )
        .with_details(Value::Object(metadata)));
    }
    Ok(obj)
}
