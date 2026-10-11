//! Reading the proxied response: a size-capped body, and the API server's own `Status` answers.

use std::future::poll_fn;
use std::pin::Pin;

use http::header::CONTENT_TYPE;
use http_body::Body as _;
use kube::client::Body;
use oxikube_domain::OxiResult;
use oxikube_ports::ProxyResponse;
use serde::Deserialize;

use crate::auth::classify;

/// The part of a Kubernetes `Status` object that tells the API server's answer from a Service's.
#[derive(Deserialize)]
struct ApiStatus {
    kind: String,
    #[serde(default)]
    message: String,
}

/// Reads at most `limit` bytes of `response`'s body; a longer one is a validation error, not a
/// stall or an unbounded allocation.
pub(super) async fn read(response: http::Response<Body>, limit: usize) -> OxiResult<ProxyResponse> {
    let (parts, body) = response.into_parts();
    let status = parts.status.as_u16();
    let content_type = parts
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let over = || ProxyResponse {
        status,
        content_type: content_type.clone(),
        body: Vec::new(),
        truncated: true,
        api_server_message: None,
    };
    if body.size_hint().lower() > limit as u64 {
        return Ok(over());
    }
    let mut body = std::pin::pin!(body);
    let mut bytes = Vec::new();
    while let Some(frame) = poll_fn(|cx| Pin::as_mut(&mut body).poll_frame(cx)).await {
        let frame = frame.map_err(|e| classify(&e))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        if bytes.len() + data.len() > limit {
            return Ok(over());
        }
        bytes.extend_from_slice(&data);
    }
    let api_server_message = api_server_message(status, content_type.as_deref(), &bytes);
    Ok(ProxyResponse {
        status,
        content_type,
        body: bytes,
        truncated: false,
        api_server_message,
    })
}

/// The `message` of a Kubernetes `Status` body on an error status: the API server's own refusal.
/// A Service answering with such an object is not a case worth telling apart.
fn api_server_message(status: u16, content_type: Option<&str>, body: &[u8]) -> Option<String> {
    if status < 400 || !content_type.is_some_and(|t| t.contains("json")) {
        return None;
    }
    let parsed: ApiStatus = serde_json::from_slice(body).ok()?;
    (parsed.kind == "Status").then_some(parsed.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_status_object_on_an_error_is_the_api_servers() {
        let json = Some("application/json");
        let status = br#"{"kind":"Status","apiVersion":"v1","message":"no endpoints available"}"#;
        assert_eq!(
            api_server_message(503, json, status).as_deref(),
            Some("no endpoints available")
        );
        // A success is the Service's, whatever it looks like.
        assert_eq!(api_server_message(200, json, status), None);
        // Prometheus' own error envelope is not a Status.
        let prom = br#"{"status":"error","errorType":"bad_data","error":"x"}"#;
        assert_eq!(api_server_message(400, json, prom), None);
        assert_eq!(api_server_message(503, Some("text/html"), status), None);
        assert_eq!(api_server_message(503, json, b"not json"), None);
    }
}
