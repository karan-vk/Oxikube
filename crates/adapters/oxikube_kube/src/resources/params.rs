//! Port [`ListOptions`] to kube `ListParams`.
//!
//! # `resourceVersion` semantics
//!
//! | `resource_version` | `version_match` | Server behaviour |
//! |---|---|---|
//! | unset | unset | consistent read from etcd (the default; always the latest) |
//! | `"0"` | unset | any version, served from the watch cache (fast, may be stale) |
//! | `"N"` | `NotOlderThan` | at least as new as `N` |
//! | `"N"` | `Exact` | exactly `N`; 410 Gone once `N` is compacted |
//!
//! # Timeout
//!
//! kube 4.2 does not send `ListParams::timeout` on a list (only `WatchParams` emits
//! `timeoutSeconds`), so [`ListOptions::timeout_secs`] is a client-side deadline per request,
//! applied by [`deadline`] around the call; elapsed maps to a `Timeout` error.
//!
//! Two rules the server enforces are applied here so the error is ours and early:
//! `version_match` needs a `resource_version`, and `Exact` needs a non-zero one (both are
//! [`Validation`](oxikube_domain::ErrorKind::Validation) errors). A continue token pins the
//! snapshot of the first page, and the server rejects an explicit `resourceVersion` alongside
//! it, so on a continuation `resource_version` / `version_match` are dropped. kube also omits
//! `resourceVersion=0` when a `limit` is set: with it the server would ignore the limit and
//! return the whole collection in one response.

use std::time::Duration;

use kube::api::{ListParams, VersionMatch as KubeMatch};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ListOptions, VersionMatch};

/// The client-side deadline for one list request: `timeout_secs`, with 0 meaning none.
pub(crate) fn deadline(options: &ListOptions) -> Option<Duration> {
    options
        .timeout_secs
        .filter(|&s| s > 0)
        .map(|s| Duration::from_secs(u64::from(s)))
}

/// Builds the kube parameters for one list request, validating the combination.
pub(crate) fn list_params(options: &ListOptions) -> OxiResult<ListParams> {
    let nonempty = |s: &Option<String>| s.clone().filter(|v| !v.is_empty());
    let continue_token = nonempty(&options.continue_token);
    let (resource_version, version_match) = if continue_token.is_some() {
        (None, None)
    } else {
        validate_version(options)?;
        (nonempty(&options.resource_version), options.version_match)
    };
    Ok(ListParams {
        label_selector: nonempty(&options.label_selector),
        field_selector: nonempty(&options.field_selector),
        // kube 4.2 never puts `ListParams::timeout` on a list request (only watches carry
        // `timeoutSeconds`), so the deadline is enforced client-side; see `deadline`.
        timeout: None,
        // 0 means "no limit" on the wire; model it as unset.
        limit: options.limit.filter(|&l| l > 0),
        continue_token,
        version_match: version_match.map(|m| match m {
            VersionMatch::NotOlderThan => KubeMatch::NotOlderThan,
            VersionMatch::Exact => KubeMatch::Exact,
        }),
        resource_version,
    })
}

fn validate_version(options: &ListOptions) -> OxiResult<()> {
    let rv = options
        .resource_version
        .as_deref()
        .filter(|v| !v.is_empty());
    match (rv, options.version_match) {
        (None, Some(_)) => Err(OxiError::validation(
            "a resource version match needs a resource version",
        )),
        (Some("0"), Some(VersionMatch::Exact)) => Err(OxiError::validation(
            "an Exact resource version match needs a non-zero resource version",
        )),
        _ => Ok(()),
    }
}
