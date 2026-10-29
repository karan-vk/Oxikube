//! Apply (E10-S08): one object's manifest to a cluster, dry run first, through the guard; and many
//! objects as one plan (E20-S03, `plan`).
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
//! | `service` | [`ApplyService`]: [`preview`](ApplyService::preview) and [`apply`](ApplyService::apply_plan), the guard's confirmation answered by the preview, errors sorted into [`ApplyError`] (conflicts with their managers and times) |
//! | `handler` | [`register_commands`]: the `resource::Apply` handler (and, through `plan`, the `resource::ApplyPlan` one) |
//! | `write` | `ObjectWrite`: the only code that writes a manifest (server-side apply under the field manager, the replace / create fallback), shared by both handlers |
//! | `plan` | [`ApplyService::plan`] / [`dry_run`](ApplyService::dry_run_plan) / [`apply`](ApplyService::apply_plan): many objects as one `resource::ApplyPlan` (E20-S03): kind order, the whole plan dry-run first, one confirmation at the riskiest object's risk, rank by rank, CRDs `Established` first, results streamed, one audit record per object under a batch id |
//! | `load` | [`load_path`] / [`load_text`]: files, directories (recursive, sorted, capped) and text read into `oxikube_domain::manifests` documents (E20-S01) |
//! | `manifest` | the manifest read as one object, its namespace, the request body |
//! | `mask` | Secret values replaced by keyed fingerprints before anything leaves this module |
//! | `batch` | [`ApplyService::preview_batch`] / [`apply_batch`](ApplyService::apply_batch): many documents (E10-S09), each the same `resource::Apply`: all dry-run first, then applied in buffer order, per-document results, a failure not stopping the rest (as `kubectl apply`) except for the documents that need it |
//! | `change` | [`Change`]: create / update / unchanged from a dry run; [`SERVER_OWNED_FIELDS`], the fields every diff ignores |
//! | `types` | [`ApplyRequest`], [`ApplyPreview`], [`ApplyReport`], [`ApplyMethod`], [`FieldConflict`] |
//!
//! Every request goes through the `MutationGuard`: blocked on a read-only cluster for every
//! initiator (and re-checked right before each request is sent), confirmed at risk `Low` (the
//! diff preview is the confirmation), audited for the dry run and for the apply, whatever their
//! outcome. The audit record names the object and the field manager, never the manifest.
//! Plan review #397: this is the one apply service; E20 extends it to many objects.

mod batch;
mod change;
mod handler;
mod load;
pub(crate) mod manifest;
pub(crate) mod mask;
mod plan;
mod service;
mod types;
pub(crate) mod write;

#[cfg(test)]
mod tests;

use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};

use crate::session::ClusterSessionManager;

pub use batch::{
    BatchCounts, BatchPreview, BatchReport, DocumentPreview, DocumentResult, DocumentStatus,
    PreviewOutcome,
};
pub use change::{Change, SERVER_OWNED_FIELDS};
pub use handler::{DEFAULT_FIELD_MANAGER, register_commands};
pub use load::{LoadLimits, LoadNote, LoadedManifests, load_path, load_text};
pub(crate) use mask::SecretMask;
pub use plan::{
    ApplyOptions, ClusterPlan, MAX_PLAN_OBJECTS, ObjectResult, ObjectStatus, PlanReport,
    PlanStream, PlanTarget,
};
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

/// The object a `resource::Apply` names, in the namespace the handler writes it to: the
/// cluster's discovery says whether the kind is namespaced, so an object no namespace is given
/// for is recorded in `default`, as it is written, and a cluster-scoped one in none. `None` for
/// any other command, a manifest that names no single object, or a cluster without a session
/// (then [`audit_target`] answers).
pub(crate) async fn resolved_audit_target(
    command: &Command,
    cluster: &ClusterId,
    sessions: &ClusterSessionManager,
) -> Option<ResourceRef> {
    let Command::ResourceApply {
        manifest,
        namespace,
        ..
    } = command
    else {
        return None;
    };
    let manifest = manifest::Manifest::parse(manifest).ok()?;
    let session = sessions.get(cluster)?;
    let namespace = manifest
        .resolve_namespace(&session, namespace.as_deref())
        .await;
    Some(manifest.target(cluster.clone(), namespace.as_deref()))
}

/// The one-line confirmation of a `resource::ApplyPlan`; `None` for any other command.
pub(crate) fn summary(command: &Command, context: &str) -> Option<String> {
    plan::summary(command, context)
}

/// What the audit record of a `resource::Apply` adds to its target: the field manager, force
/// and the reviewed version (or `reviewed=create`); for a `resource::ApplyPlan`, the counts and
/// kinds. Never the manifest.
pub(crate) fn audit_detail(command: &Command) -> Option<String> {
    if let Some(detail) = plan::audit_detail(command) {
        return Some(detail);
    }
    let Command::ResourceApply {
        field_manager,
        force,
        resource_version,
        reviewed_absent,
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
    if *reviewed_absent {
        detail.push_str(" reviewed=create");
    }
    Some(detail)
}
