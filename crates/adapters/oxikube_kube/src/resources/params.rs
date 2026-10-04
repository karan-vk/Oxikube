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
//! Two rules the server enforces are applied here so the error is ours and early:
//! `version_match` needs a `resource_version`, and `Exact` needs a non-zero one (both are
//! [`Validation`](oxikube_domain::ErrorKind::Validation) errors). A continue token pins the
//! snapshot of the first page, and the server rejects an explicit `resourceVersion` alongside
//! it, so on a continuation `resource_version` / `version_match` are dropped. kube also omits
//! `resourceVersion=0` when a `limit` is set: with it the server would ignore the limit and
//! return the whole collection in one response.

use kube::api::{ListParams, VersionMatch as KubeMatch};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ListOptions, VersionMatch};

/// Builds the kube parameters for one list request, validating the combination.
pub(super) fn list_params(options: &ListOptions) -> OxiResult<ListParams> {
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
        timeout: options.timeout_secs,
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
