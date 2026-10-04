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
//! talks TLS to the proxy itself with the same rustls setup) and `socks5://`. Basic
//! auth in the URL's userinfo is sent to `http`/`https` proxies. Anything else is an
//! [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) error naming the scheme; a
//! URL without a scheme is a [`Validation`](oxikube_domain::ErrorKind::Validation) error.
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
    let Some(raw) = from_cluster.or(env.https_proxy()) else {
        return Ok(None);
    };
    let uri: http::Uri = normalize_scheme(raw)
        .parse()
        .map_err(|err| classify_kubeconfig(&KubeconfigError::ParseProxyUrl(err)))?;
    match uri.scheme_str() {
        Some("http" | "https" | "socks5") => {}
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
