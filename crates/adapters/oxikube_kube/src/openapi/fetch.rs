//! The three GETs the adapter makes: the index, the server version, one group document.

use std::time::Duration;

use kube::Client;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use tracing::debug;

use super::index::{Index, parse_index};
use crate::auth::classify;

/// `GET /openapi/v3`: the index of group-version documents. A 404 means the
/// server predates OpenAPI v3: [`Unsupported`](ErrorKind::Unsupported), never
/// swallowed.
pub(super) async fn fetch_index(client: &Client, timeout: Duration) -> OxiResult<Index> {
    let text = get(client, "/openapi/v3", timeout).await.map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            no_openapi_v3()
        } else {
            err
        }
    })?;
    let document: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| OxiError::internal(format!("openapi: index is not JSON: {e}")))?;
    Ok(parse_index(&document))
}

/// The error for a server without `/openapi/v3`.
pub(super) fn no_openapi_v3() -> OxiError {
    OxiError::unsupported("the API server has no /openapi/v3 endpoint")
}

/// `GET /version`: the server's `gitVersion`, which keys the disk cache. `None`
/// when it cannot be read (the cache then falls back to `unknown`, still
/// validated by the index hash).
pub(super) async fn fetch_server_version(client: &Client, timeout: Duration) -> Option<String> {
    let text = get(client, "/version", timeout).await.ok()?;
    let document: serde_json::Value = serde_json::from_str(&text).ok()?;
    document
        .get("gitVersion")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// `GET` one group-version document as raw text. Bodies are public API
/// schemas, but only their size is ever logged.
pub(super) async fn fetch_document(
    client: &Client,
    url: &str,
    timeout: Duration,
) -> OxiResult<String> {
    let text = get(client, url, timeout).await?;
    debug!(url = %path_for_logs(url), bytes = text.len(), "openapi: group document fetched");
    Ok(text)
}

/// `GET` `url` as text within `timeout`. The URL is server-built (index) or a
/// fixed path; error messages carry the path but never a body.
async fn get(client: &Client, url: &str, timeout: Duration) -> OxiResult<String> {
    let request = http::Request::get(url)
        .body(Vec::new())
        .map_err(|e| OxiError::validation(format!("openapi: bad request path: {e}")))?;
    tokio::time::timeout(timeout, client.request_text(request))
        .await
        .map_err(|_| OxiError::timeout(format!("openapi: GET {url} did not complete in time")))?
        .map_err(|e| classify_status(&e))
}

/// Maps a fetch failure: 404 is `NotFound` (the caller refines the index case
/// to `Unsupported`); everything else follows the adapter's table.
fn classify_status(err: &kube::Error) -> OxiError {
    match err {
        kube::Error::Api(status) if status.code == 404 => {
            OxiError::not_found("the API server has no such OpenAPI v3 document")
        }
        _ => classify(err),
    }
}

/// The URL path without its query, for short stable logs.
fn path_for_logs(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}
