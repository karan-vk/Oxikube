//! Proxy selection and validation for a context.
//!
//! # Precedence (highest first)
//!
//! 1. the cluster's `proxy-url` in the kubeconfig;
//! 2. the environment fallback captured in [`ProxyEnv`]: `HTTPS_PROXY`, then `https_proxy`;
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
//! Not honoured, and deliberately not part of this story: kube ignores it, and it only
//! matters for the environment fallback (an explicit kubeconfig `proxy-url` is a
//! per-cluster choice). Until it is supported, a cluster that must bypass `HTTPS_PROXY`
//! needs the variable unset. Follow-up: match `NO_PROXY` (hosts, domain suffixes, CIDRs,
//! `*`) against the cluster server for the environment fallback only.

use kube::config::KubeconfigError;
use oxikube_domain::OxiError;

use super::build::ProxyEnv;
use super::entry::ContextDefinition;
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
