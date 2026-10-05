//! Table list and watch requests: kube's `Request` builders plus the Table `Accept` header
//! and `includeObject`.
//!
//! kube 4.2's `kube::core::Request::{list, watch}` build the URL and query from
//! `ListParams` / `WatchParams` but set no `Accept` header (the server then sends JSON), so
//! we add `Accept` ourselves and append `includeObject`. The Accept list ends with plain
//! `application/json`: a server that cannot produce a Table (some aggregated APIs) answers
//! with the ordinary list instead of 406, which the feed detects by `kind`.
//!
//! `v1beta1` Tables (servers older than 1.15) are not offered: every supported cluster
//! serves `meta.k8s.io/v1`, and an old server falls back to plain JSON like any other.

use http::header::{ACCEPT, HeaderValue};
use kube::api::{ListParams, WatchParams};
use kube::core::Request;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::IncludeObject;

/// `Accept` for Table requests: a `meta.k8s.io/v1` Table, else plain JSON.
pub const TABLE_ACCEPT: &str = "application/json;as=Table;v=v1;g=meta.k8s.io,application/json";

/// A Table list request for the collection at `path` (`/api/v1/namespaces/x/pods`).
pub(crate) fn list(
    path: &str,
    params: &ListParams,
    include: IncludeObject,
) -> OxiResult<http::Request<Vec<u8>>> {
    let request = Request::new(path).list(params).map_err(build_error)?;
    Ok(as_table(request, include))
}

/// A Table watch request for the collection at `path`, from `resource_version`.
pub(crate) fn watch(
    path: &str,
    params: &WatchParams,
    resource_version: &str,
    include: IncludeObject,
) -> OxiResult<http::Request<Vec<u8>>> {
    let request = Request::new(path)
        .watch(params, resource_version)
        .map_err(build_error)?;
    Ok(as_table(request, include))
}

/// Adds `includeObject` to the query and the Table `Accept` header.
fn as_table(request: http::Request<Vec<u8>>, include: IncludeObject) -> http::Request<Vec<u8>> {
    let (mut parts, body) = request.into_parts();
    let uri = parts.uri.to_string();
    // kube always ends the path with `?`, then `&`-joined pairs; be tolerant of either.
    let separator = match uri.chars().last() {
        Some('?' | '&') => "",
        _ if uri.contains('?') => "&",
        _ => "?",
    };
    let with_include = format!("{uri}{separator}includeObject={}", include.as_str());
    // Appending one ASCII pair to a URI kube built cannot make it invalid; keep the original
    // rather than fail if it somehow did.
    if let Ok(uri) = with_include.parse() {
        parts.uri = uri;
    }
    parts
        .headers
        .insert(ACCEPT, HeaderValue::from_static(TABLE_ACCEPT));
    http::Request::from_parts(parts, body)
}

fn build_error(err: kube::core::request::Error) -> OxiError {
    OxiError::validation(format!("invalid table request: {err}"))
}
