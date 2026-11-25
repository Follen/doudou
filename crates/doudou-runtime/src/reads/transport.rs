//! Page-context HTTP JSON transport wrapping [`doudou_cdp::ChromeSession::page_fetch`]
//! and [`doudou_protocol::parse_json_response`].

use std::time::Duration;

use doudou_cdp::{ChromeSession, FetchRequest, FetchResponse as CdpFetchResponse};
use doudou_core::Result;
use doudou_protocol::{FetchResponse as ProtoFetchResponse, ResponseCodes, parse_json_response};
use serde_json::{Map, Value};

/// Convert a [`doudou_cdp::FetchResponse`] into [`doudou_protocol::FetchResponse`].
pub fn to_protocol_fetch_response(resp: &CdpFetchResponse) -> ProtoFetchResponse {
    ProtoFetchResponse {
        http_status: resp.status,
        content_type: resp
            .headers
            .get("content-type")
            .cloned()
            .unwrap_or_default(),
        retry_after: resp.headers.get("retry-after").cloned(),
        text: resp.body_text.clone().unwrap_or_default(),
    }
}

/// Classify a raw page-context response using live login/verification status from `session`.
pub async fn parse_result(
    session: &mut ChromeSession,
    response: &ProtoFetchResponse,
    codes: ResponseCodes,
    operation: &str,
) -> Result<Map<String, Value>> {
    let login = session.classify_login().await?;
    parse_json_response(
        response,
        codes,
        login.valid,
        login.verification_visible,
        operation,
    )
}

/// Fetch JSON via in-page `fetch()` with bounded timeout and optional single retry on
/// `UPSTREAM_UNAVAILABLE`.
pub async fn fetch_json(
    session: &mut ChromeSession,
    url: &str,
    codes: ResponseCodes,
    operation: &str,
    timeout_ms: u64,
    retry_upstream_once: bool,
) -> Result<Map<String, Value>> {
    let attempts = if retry_upstream_once { 2 } else { 1 };
    let request = FetchRequest::get_text(url).with_timeout_ms(timeout_ms);

    for attempt in 1..=attempts {
        let raw = session.page_fetch(&request).await?;
        let proto = to_protocol_fetch_response(&raw);
        match parse_result(session, &proto, codes, operation).await {
            Ok(parsed) => return Ok(parsed),
            Err(err) => {
                if err.code != "UPSTREAM_UNAVAILABLE" || attempt == attempts {
                    return Err(err);
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
    unreachable!("attempt loop always returns on final iteration")
}
