//! Apply (E10-S08): one object's manifest to a cluster, dry run first, through the guard.
//!
//! ```text
//! editor ─preview─> ApplyService ─resource::Apply (dry run)─> guard ─> handler ─> SSA dry run
//!        <─ApplyPreview (live + result, Secret values masked)──────────────────────┘
//! user confirms the diff preview
//! editor ─apply──> ApplyService ─resource::Apply──────────────> guard ─> handler ─> SSA dry run, SSA
//!                                                                        (415: replace at the reviewed version)
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `service` | [`ApplyService`]: [`preview`](ApplyService::preview) and [`apply`](ApplyService::apply), the guard's confirmation answered by the preview, errors sorted into [`ApplyError`] (conflicts with their managers and times) |
//! | `handler` | [`register_commands`]: the `resource::Apply` handler, the only code that writes a manifest (server-side apply under the field manager, the replace / create fallback) |
//! | `manifest` | the manifest read as one object, its namespace, the request body |
//! | `mask` | Secret values replaced by keyed fingerprints before anything leaves this module |
//! | `types` | [`ApplyRequest`], [`ApplyPreview`], [`ApplyReport`], [`ApplyMethod`], [`FieldConflict`] |
//!
//! Every request goes through the `MutationGuard`: blocked on a read-only cluster for every
//! initiator (and re-checked right before each request is sent), confirmed at risk `Low` (the
//! diff preview is the confirmation), audited for the dry run and for the apply, whatever their
//! outcome. The audit record names the object and the field manager, never the manifest.
//! Plan review #397: this is the one apply service; E20 extends it to many objects.

mod handler;
mod manifest;
mod mask;
mod service;
mod types;

#[cfg(test)]
mod tests;

use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};

pub use handler::{DEFAULT_FIELD_MANAGER, register_commands};
pub use service::ApplyService;
pub use types::{
    ApplyError, ApplyMethod, ApplyPreview, ApplyReport, ApplyRequest, FieldConflict,
    conflict_managers,
};

/// The object a `resource::Apply` names, for its audit record; `None` for any other command or a
/// manifest that names no single object.
pub(crate) fn audit_target(command: &Command, cluster: &ClusterId) -> Option<ResourceRef> {
    let Command::ResourceApply {
        manifest,
        namespace,
        ..
    } = command
    else {
        return None;
    };
    manifest::audit_target(cluster, manifest, namespace.as_deref())
}

/// What the audit record of a `resource::Apply` adds to its target: the field manager, force
/// and the reviewed version. Never the manifest.
pub(crate) fn audit_detail(command: &Command) -> Option<String> {
    let Command::ResourceApply {
        field_manager,
        force,
        resource_version,
        ..
    } = command
    else {
        return None;
    };
    let manager = field_manager.as_deref().unwrap_or(DEFAULT_FIELD_MANAGER);
    let mut detail = format!("manager={manager} force={force}");
    if let Some(version) = resource_version {
        detail.push_str(&format!(" resource_version={version}"));
    }
    Some(detail)
}
