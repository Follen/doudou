//! Page-generated write request field validation and response classification.

use std::collections::BTreeMap;

use doudou_core::{DoudouError, Result, sha256_bytes};
use serde_json::{Map, Value, json};
use url::Url;

/// Decode query and JSON/form request fields used to verify page-generated writes.
pub fn request_fields(raw_url: &str, body: &[u8]) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    if let Ok(parsed) = Url::parse(raw_url) {
        for (k, v) in parsed.query_pairs() {
            fields.insert(k.into_owned(), v.into_owned());
        }
    }
    if !body.is_empty() {
        let decoded = String::from_utf8_lossy(body);
        if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&decoded) {
            for (k, v) in map {
                match v {
                    Value::String(s) => {
                        fields.insert(k, s);
                    }
                    Value::Number(n) => {
                        fields.insert(k, n.to_string());
                    }
                    Value::Bool(b) => {
                        fields.insert(k, b.to_string());
                    }
                    _ => {}
                }
            }
        } else {
            for (k, v) in url::form_urlencoded::parse(body) {
                fields.insert(k.into_owned(), v.into_owned());
            }
        }
    }
    fields
}

/// Verify that a captured comment write/like request matches the confirmed plan fields.
pub fn validate_comment_request(
    fields: &BTreeMap<String, String>,
    aweme_id: &str,
    text: Option<&str>,
    comment_id: Option<&str>,
    digg_type: Option<&str>,
) -> Result<BTreeMap<String, String>> {
    let actual_aweme = fields
        .get("aweme_id")
        .or_else(|| fields.get("item_id"))
        .map(String::as_str);
    let actual_text = fields.get("text").map(String::as_str);
    let actual_comment = fields
        .get("comment_id")
        .or_else(|| fields.get("cid"))
        .or_else(|| fields.get("reply_id"))
        .map(String::as_str);
    let actual_digg = fields.get("digg_type").map(String::as_str);

    let mut mismatches = Vec::new();
    if actual_aweme != Some(aweme_id) {
        mismatches.push("aweme_id");
    }
    if let Some(expected_text) = text
        && actual_text != Some(expected_text)
    {
        mismatches.push("text");
    }
    if let Some(expected_cid) = comment_id
        && actual_comment != Some(expected_cid)
    {
        mismatches.push("comment_id");
    }
    if let Some(expected_digg) = digg_type
        && actual_digg != Some(expected_digg)
    {
        mismatches.push("digg_type");
    }
    if !mismatches.is_empty() {
        return Err(DoudouError::new(
            "PAGE_WRITE_REQUEST_DRIFTED",
            "The page-generated write request did not match the confirmed plan.",
        )
        .with_details(json!({
            "write_started": true,
            "mismatched_fields": mismatches,
        })));
    }

    let mut verified = BTreeMap::new();
    verified.insert("aweme_id".to_owned(), aweme_id.to_owned());
    if let Some(t) = actual_text
        && text.is_some()
    {
        verified.insert("text".to_owned(), t.to_owned());
    }
    if let Some(c) = actual_comment
        && comment_id.is_some()
    {
        verified.insert("comment_id".to_owned(), c.to_owned());
    }
    if let Some(d) = actual_digg
        && digg_type.is_some()
    {
        verified.insert("digg_type".to_owned(), d.to_owned());
    }
    Ok(verified)
}

/// Classify a captured page-generated write response body and HTTP status.
pub fn parse_page_write_response(
    body: &[u8],
    http_status: u16,
    operation: &str,
) -> Result<Map<String, Value>> {
    let mut metadata = Map::new();
    metadata.insert("write_started".to_owned(), json!(true));
    metadata.insert("operation".to_owned(), json!(operation));
    metadata.insert("http_status".to_owned(), json!(http_status));
    metadata.insert("response_bytes".to_owned(), json!(body.len()));
    metadata.insert("response_sha256".to_owned(), json!(sha256_bytes(body)));

    if body.is_empty() {
        return Err(DoudouError::new(
            "WRITE_RESULT_UNKNOWN",
            "The page write returned an empty response.",
        )
        .with_details(Value::Object(metadata)));
    }
    let parsed: Value = serde_json::from_slice(body).map_err(|_| {
        DoudouError::new(
            "WRITE_RESULT_UNKNOWN",
            "The page write returned a non-JSON response.",
        )
        .with_details(Value::Object(metadata.clone()))
    })?;
    let Value::Object(obj) = parsed else {
        return Err(DoudouError::new(
            "WRITE_RESULT_UNKNOWN",
            "The page write response was not an object.",
        )
        .with_details(Value::Object(metadata)));
    };

    let decision = obj
        .get("decision")
        .and_then(Value::as_str)
        .or_else(|| {
            obj.get("data")
                .and_then(|d| d.get("decision"))
                .and_then(Value::as_str)
        })
        .unwrap_or("");
    if decision.eq_ignore_ascii_case("KICK") {
        metadata.insert("result_known".to_owned(), json!(true));
        metadata.insert("gateway_decision".to_owned(), json!("KICK"));
        return Err(DoudouError::new(
            "WRITE_VERIFICATION_REQUIRED",
            "The page write reached account verification.",
        )
        .with_details(Value::Object(metadata)));
    }

    let status_code = obj.get("status_code").or_else(|| obj.get("statusCode"));
    let business_ok = match status_code {
        None | Some(Value::Null) => true,
        Some(Value::Number(n)) => n.as_i64() == Some(0),
        _ => false,
    };
    if !(200..300).contains(&http_status) || !business_ok {
        metadata.insert("result_known".to_owned(), json!(true));
        metadata.insert(
            "status_code".to_owned(),
            status_code.cloned().unwrap_or(Value::Null),
        );
        return Err(DoudouError::new(
            "VIDEO_WRITE_FAILED",
            "The page write returned a business error.",
        )
        .with_details(Value::Object(metadata)));
    }
    Ok(obj)
}

/// Extract the created comment object from a publish response payload.
pub fn response_comment(value: &Map<String, Value>) -> Option<&Map<String, Value>> {
    value.get("comment").and_then(Value::as_object).or_else(|| {
        value
            .get("data")
            .and_then(Value::as_object)
            .and_then(|d| d.get("comment"))
            .and_then(Value::as_object)
    })
}
