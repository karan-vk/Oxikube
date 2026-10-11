//! [`set_unschedulable`]: cordon and uncordon.

use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{Patch, WriteOptions};
use serde_json::json;

use crate::guard::Mutation;

/// The merge patch that sets (`true`) or clears (`false`) `spec.unschedulable`. Clearing sends
/// `null`, which removes the field (what `kubectl uncordon` does), rather than `false`.
fn unschedulable_patch(unschedulable: bool) -> Patch {
    Patch::merge(json!({ "spec": { "unschedulable": unschedulable.then_some(true) } }))
}

/// What a cordon or uncordon did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeOutcome {
    /// The patch was applied.
    Applied,
    /// A dry run: the server accepted the patch and nothing changed.
    DryRun,
}

/// Marks the node unschedulable (`unschedulable`) or schedulable again, through the guard's write
/// permission: a server dry run of the patch first, the real one only after it passed.
///
/// # Errors
///
/// A validation error, before any request, when `node` is not a cluster-scoped Node (the
/// commands are open to agents, which may name any object); otherwise whatever the API server
/// answers (`NotFound`, `Forbidden`, admission).
pub async fn set_unschedulable(
    mutation: &Mutation,
    node: &ResourceRef,
    unschedulable: bool,
) -> OxiResult<NodeOutcome> {
    if !node.gvk.is_node() || node.namespace.is_some() {
        return Err(OxiError::validation(format!(
            "cordon and uncordon need a node, not a {}",
            node.gvk.kind
        )));
    }
    let patch = unschedulable_patch(unschedulable);
    let kind: &Gvk = &node.gvk;
    let writer = mutation.writer();
    let rehearsal = WriteOptions {
        dry_run: true,
        ..mutation.write_options()
    };
    writer
        .patch(kind, None, &node.name, &patch, &rehearsal)
        .await?;
    if mutation.dry_run() {
        return Ok(NodeOutcome::DryRun);
    }
    writer
        .patch(kind, None, &node.name, &patch, &mutation.write_options())
        .await?;
    Ok(NodeOutcome::Applied)
}
