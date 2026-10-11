//! Building the API server request for a [`ProxyRequest`].

use http::header::{ACCEPT, CONTENT_TYPE};
use kube::client::Body;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ProxyMethod, ProxyRequest, ProxyScheme, ServiceRef};
use url::form_urlencoded;

/// A Kubernetes name or port name: what may stand between two `/` or `:` of the proxy path.
fn is_name(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 253
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// The path on the Service: absolute, no dot segments, no query or fragment, only characters
/// that are safe to put in a URL path unescaped.
fn is_service_path(path: &str) -> bool {
    path.starts_with('/')
        && !path
            .split('/')
            .any(|segment| segment == ".." || segment == ".")
        && path.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | '~' | ':' | '@')
        })
}

/// `/api/v1/namespaces/<ns>/services/[https:]<name>:<port>/proxy<path>`.
pub(super) fn proxy_path(service: &ServiceRef, path: &str) -> OxiResult<String> {
    let port = service.port.to_string();
    for (what, part) in [
        ("namespace", service.namespace.as_str()),
        ("service name", service.name.as_str()),
        ("port", port.as_str()),
    ] {
        if !is_name(part) {
            return Err(OxiError::validation(format!(
                "`{part}` is not a valid {what} for the service proxy"
            )));
        }
    }
    if !is_service_path(path) {
        return Err(OxiError::validation(format!(
            "`{path}` is not a valid path on a proxied service"
        )));
    }
    let scheme = match service.scheme {
        ProxyScheme::Http => "",
        ProxyScheme::Https => "https:",
    };
    Ok(format!(
        "/api/v1/namespaces/{}/services/{scheme}{}:{port}/proxy{path}",
        service.namespace, service.name
    ))
}

/// The HTTP request: a `GET` with the parameters in the query string, or a form `POST`.
pub(super) fn build(
    service: &ServiceRef,
    request: &ProxyRequest,
) -> OxiResult<http::Request<Body>> {
    let path = proxy_path(service, &request.path)?;
    let encoded = || {
        form_urlencoded::Serializer::new(String::new())
            .extend_pairs(request.params.iter())
            .finish()
    };
    let built = match request.method {
        ProxyMethod::Get => {
            let uri = if request.params.is_empty() {
                path
            } else {
                format!("{path}?{}", encoded())
            };
            http::Request::get(uri)
                .header(ACCEPT, "application/json")
                .body(Body::empty())
        }
        ProxyMethod::PostForm => http::Request::post(path)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(encoded().into_bytes())),
    };
    built.map_err(|e| OxiError::validation(format!("cannot build the proxy request: {e}")))
}
