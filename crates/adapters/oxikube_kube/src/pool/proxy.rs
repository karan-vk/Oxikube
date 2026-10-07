//! Proxy selection and validation for a context.
//!
//! # Precedence (highest first)
//!
//! 1. the cluster's `proxy-url` in the kubeconfig;
//! 2. the environment fallback captured in [`ProxyEnv`]: `HTTPS_PROXY`, then `https_proxy`,
//!    unless the cluster's `server` host matches `NO_PROXY` / `no_proxy` (see below);
//! 3. no proxy.
//!
//! An empty value counts as unset at every step.
//!
//! # Schemes
//!
//! kube 4.2 (features `http-proxy` and `socks5`, both on in the workspace) supports
//! `http://` and `https://` proxies through an HTTP `CONNECT` tunnel (the `https` form
//! talks TLS to the proxy itself) and `socks5://`. Basic
//! auth in the URL's userinfo is sent to `http`/`https` proxies. Anything else is an
//! [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) error naming the scheme.
//!
//! SOCKS5 credentials are not supported: kube 4.2 builds its SOCKS5 connector without
//! `with_auth`, and hyper-util never reads the URL's userinfo, so the handshake offers
//! only "no authentication" and a proxy that wants a password fails every connection
//! with an opaque error. A `socks5://user:pass@host` URL is therefore rejected up front
//! as `Unsupported` (without echoing the URL). Sending the credentials would need our
//! own connector; until then, use an unauthenticated SOCKS5 proxy or an `http(s)` one.
//!
//! A kubeconfig `proxy-url` without a scheme is a
//! [`Validation`](oxikube_domain::ErrorKind::Validation) error, as in client-go. The
//! environment fallback is more lenient, again matching client-go (Go's
//! `httpproxy.parseProxy`): a value without `://` (such as `HTTPS_PROXY=proxy.corp:3128`)
//! is read as `http://` plus the value.
//!
//! For an `https://` proxy kube builds the proxy leg from the same [`Config`](kube::Config)
//! as the API connection: the cluster CA verifies the proxy's certificate, `tls-server-name`
//! is the SNI and name checked for the proxy (not only for the API server),
//! `insecure-skip-tls-verify` also disables verification of the proxy, and the kubeconfig
//! client certificate is presented to the proxy.
//!
//! The scheme is matched case-insensitively and `socks5h://` is accepted as `socks5://`:
//! kube's SOCKS5 connector hands the target hostname to the proxy for resolution, which
//! is what `socks5h` means. URLs are never echoed in errors (they may carry credentials).
//!
//! # `NO_PROXY`
//!
//! `NO_PROXY`, then `no_proxy`, is captured in [`ProxyEnv`] next to `HTTPS_PROXY` (so tests
//! never read process environment) and consulted for the environment fallback only: a
//! kubeconfig `proxy-url` is an explicit per-cluster choice and is used even when the
//! cluster's host is listed. When the `server` host matches, the environment proxy is
//! skipped (and not even validated) and the cluster connects directly. The list syntax
//! (hosts, domain suffixes with or without a leading dot, IPs, CIDRs, `*`, optional
//! ports) and its matching rules are documented in `pool::no_proxy`. A `server` URL that
//! does not parse never matches, so it keeps the environment proxy.

use kube::config::KubeconfigError;
use oxikube_domain::OxiError;

use super::build::ProxyEnv;
use super::entry::ContextDefinition;
use super::no_proxy::Target;
use crate::auth::classify_kubeconfig;

/// The proxy URL to use for `definition`, validated, or `None`.
pub(super) fn resolve(
    definition: &ContextDefinition,
    env: &ProxyEnv,
) -> Result<Option<http::Uri>, OxiError> {
    let from_cluster = definition
        .cluster()
        .and_then(|c| c.proxy_url.as_deref())
        .filter(|url| !url.is_empty());
    let raw = match (from_cluster, env.https_proxy()) {
        (Some(url), _) => normalize_scheme(url),
        (None, Some(_)) if bypassed_by_no_proxy(definition, env) => return Ok(None),
        (None, Some(url)) => normalize_scheme(&with_default_scheme(url)),
        (None, None) => return Ok(None),
    };
    let uri: http::Uri = raw
        .parse()
        .map_err(|err| classify_kubeconfig(&KubeconfigError::ParseProxyUrl(err)))?;
    match uri.scheme_str() {
        Some("http" | "https") => {}
        Some("socks5") => {
            if uri.authority().is_some_and(|a| a.as_str().contains('@')) {
                return Err(OxiError::unsupported(
                    "SOCKS5 proxy authentication (user:password in the proxy URL) is not \
                     supported; use an unauthenticated SOCKS5 proxy or an http(s) proxy",
                ));
            }
        }
        Some(other) => {
            return Err(OxiError::unsupported(format!(
                "the proxy scheme `{other}` is not supported (use http, https or socks5)"
            )));
        }
        None => {
            return Err(OxiError::validation(
                "the proxy URL needs a scheme: http://, https:// or socks5://",
            ));
        }
    }
    if uri.host().is_none_or(str::is_empty) {
        return Err(OxiError::validation("the proxy URL has no host"));
    }
    Ok(Some(uri))
}

/// Whether the cluster's `server` host is listed in `NO_PROXY`.
fn bypassed_by_no_proxy(definition: &ContextDefinition, env: &ProxyEnv) -> bool {
    let Some(url) = definition.server_url() else {
        return false;
    };
    let (Some(host), Some(port)) = (Target::from_url(&url), url.port_or_known_default()) else {
        return false;
    };
    env.no_proxy().matches(&host, port)
}

/// client-go's leniency for the environment proxy: a value without `://` is read as
/// `http://` plus the value (Go `httpproxy.parseProxy`).
fn with_default_scheme(raw: &str) -> String {
    if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("http://{raw}")
    }
}

/// Lowercases the scheme and maps `socks5h` to `socks5`; the rest is untouched.
fn normalize_scheme(raw: &str) -> String {
    let Some((scheme, rest)) = raw.split_once("://") else {
        return raw.to_owned();
    };
    let scheme = scheme.to_ascii_lowercase();
    let scheme = if scheme == "socks5h" {
        "socks5"
    } else {
        &scheme
    };
    format!("{scheme}://{rest}")
}
