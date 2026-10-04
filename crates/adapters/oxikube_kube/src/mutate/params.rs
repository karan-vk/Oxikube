//! Port options to kube parameters, and the request-shape rules that need no server.

use kube::api::{DeleteParams, ListParams, PatchParams, PostParams};
use kube::api::{Preconditions as KubePreconditions, PropagationPolicy as KubePropagation};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    DeleteOptions, ListOptions, Patch, PatchKind, PropagationPolicy, WriteOptions,
};
use serde_json::Value;

/// The field manager recorded in `managedFields` when the caller names none.
pub const DEFAULT_FIELD_MANAGER: &str = "oxikube";

/// The longest field manager name the API server accepts.
const MAX_MANAGER_LEN: usize = 128;

fn manager(requested: Option<&str>) -> OxiResult<String> {
    let name = requested
        .filter(|m| !m.is_empty())
        .unwrap_or(DEFAULT_FIELD_MANAGER);
    if name.len() > MAX_MANAGER_LEN {
        return Err(OxiError::validation(format!(
            "a field manager name is at most {MAX_MANAGER_LEN} characters"
        )));
    }
    Ok(name.to_owned())
}

/// `PostParams` for create and replace.
pub(super) fn post_params(options: &WriteOptions) -> OxiResult<PostParams> {
    Ok(PostParams {
        dry_run: options.dry_run,
        field_manager: Some(manager(options.field_manager.as_deref())?),
    })
}

/// `PatchParams` and the kube patch for `patch`.
///
/// Apply: the patch's manager wins, then `options.field_manager`, then the default; `force`
/// is sent only for apply (the server rejects it elsewhere). JSON patch: the body must be an
/// RFC 6902 operation array.
pub(super) fn patch_request<'a>(
    patch: &'a Patch,
    options: &WriteOptions,
) -> OxiResult<(PatchParams, kube::api::Patch<&'a Value>)> {
    let mut params = PatchParams {
        dry_run: options.dry_run,
        ..PatchParams::default()
    };
    let body = &patch.body;
    let kube_patch = match &patch.kind {
        PatchKind::Merge => kube::api::Patch::Merge(body),
        PatchKind::Strategic => kube::api::Patch::Strategic(body),
        PatchKind::Json => {
            let ops: json_patch::Patch = serde_json::from_value(body.clone()).map_err(|_| {
                OxiError::validation("a JSON patch is an array of RFC 6902 operations")
            })?;
            kube::api::Patch::Json(ops)
        }
        PatchKind::Apply { manager: m, force } => {
            params.force = *force;
            let requested = Some(m.as_str())
                .filter(|m| !m.is_empty())
                .or(options.field_manager.as_deref());
            params.field_manager = Some(manager(requested)?);
            kube::api::Patch::Apply(body)
        }
    };
    if params.field_manager.is_none() {
        params.field_manager = Some(manager(options.field_manager.as_deref())?);
    }
    Ok((params, kube_patch))
}

/// `DeleteParams` for delete and delete-collection.
pub(super) fn delete_params(options: &DeleteOptions) -> DeleteParams {
    DeleteParams {
        dry_run: options.dry_run,
        grace_period_seconds: options.grace_period_secs,
        propagation_policy: options.propagation.map(|p| match p {
            PropagationPolicy::Orphan => KubePropagation::Orphan,
            PropagationPolicy::Background => KubePropagation::Background,
            PropagationPolicy::Foreground => KubePropagation::Foreground,
        }),
        preconditions: options.preconditions.as_ref().map(|p| KubePreconditions {
            resource_version: p.resource_version.clone(),
            uid: p.uid.clone(),
        }),
    }
}

/// The selector half of `delete_collection`: labels and fields only. A delete is not paged
/// and has no read consistency to choose, so the rest of `selection` is ignored.
pub(super) fn selection_params(selection: &ListOptions) -> ListParams {
    let nonempty = |s: &Option<String>| s.clone().filter(|v| !v.is_empty());
    ListParams {
        label_selector: nonempty(&selection.label_selector),
        field_selector: nonempty(&selection.field_selector),
        ..ListParams::default()
    }
}
