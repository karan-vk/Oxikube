//! The `pvc::Resize` handler.

use std::sync::Arc;

use oxikube_domain::command::Command;
use oxikube_domain::view::storage::{Expansion, PvcSummary, check_resize};
use oxikube_domain::{OxiError, OxiResult, Quantity};
use oxikube_ports::{Patch, PatchKind, ResourceReader, WriteOptions};
use serde_json::{Value, json};

use crate::command_bus::{CommandOutput, HandlerContext};
use crate::session::ClusterSessionManager;
use crate::storage::read::{claim_gvk, read_claim};

pub(super) async fn handle(
    command: Command,
    cx: HandlerContext,
    sessions: ClusterSessionManager,
) -> OxiResult<CommandOutput> {
    let Command::PvcResize { target, size, from } = command else {
        return Err(OxiError::validation("not a pvc::Resize command"));
    };
    if !target.gvk.group.is_empty() || &*target.gvk.kind != "PersistentVolumeClaim" {
        return Err(OxiError::validation(format!(
            "pvc::Resize resizes a PersistentVolumeClaim, not a {}",
            target.gvk.kind
        )));
    }
    let mutation = cx.require_mutation()?;
    let reader: Arc<dyn ResourceReader> = sessions
        .get(&target.cluster)
        .and_then(|session| session.resources())
        .ok_or_else(|| OxiError::not_found("the cluster is not connected"))?;
    let ns = target.namespace();

    // The live claim decides, not what the caller saw.
    let (claim, class) = read_claim(&*reader, ns, &target.name).await?;
    let current = claim.current_request();
    if let Some(seen) = from.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let seen = Quantity::parse(seen)
            .map_err(|_| OxiError::validation(format!("{seen:?} is not a size")))?;
        if current != Some(seen) {
            return Err(OxiError::conflict(format!(
                "{} changed from {seen} to {}: reopen the resize to see the current size",
                target.name,
                current.map_or_else(|| "no size".to_owned(), |q| q.to_string()),
            )));
        }
    }
    let expansion = Expansion::of(claim.storage_class.as_deref(), class.as_ref());
    let new = check_resize(current.as_ref(), &size, &expansion)
        .map_err(|block| OxiError::validation(block.to_string()))?;

    let patch = Patch {
        kind: PatchKind::Merge,
        body: json!({"spec": {"resources": {"requests": {"storage": new.to_string()}}}}),
    };
    let ns_name = format!("{}/{}", ns.unwrap_or_default(), target.name);
    let from_text = current.map(|q| q.to_string());
    let writer = mutation.writer();
    let options = mutation.write_options();
    let rehearsal = WriteOptions {
        dry_run: true,
        ..options.clone()
    };
    // The server rehearses the patch first (admission, RBAC, the class's own refusal), unless
    // the guard found it cannot dry-run one.
    if mutation.rehearses_first() {
        let rehearsed = writer
            .patch(&claim_gvk(), ns, &target.name, &patch, &rehearsal)
            .await?;
        if mutation.dry_run() {
            return Ok(CommandOutput {
                message: Some(format!("Dry run: {ns_name} can be resized to {new}")),
                data: Some(report("dry_run", from_text.as_deref(), &new, &rehearsed)),
            });
        }
    }
    let patched = writer
        .patch(&claim_gvk(), ns, &target.name, &patch, &options)
        .await?;
    Ok(CommandOutput {
        message: Some(format!(
            "Resizing {ns_name} from {} to {new}",
            from_text.as_deref().unwrap_or("its current size")
        )),
        data: Some(report("resized", from_text.as_deref(), &new, &patched)),
    })
}

/// The result as structured data: sizes and the claim's conditions after the patch (a
/// `FileSystemResizePending` condition means the volume grew and the node still has to grow the
/// file system). Nothing in it is secret.
fn report(
    outcome: &str,
    from: Option<&str>,
    to: &Quantity,
    patched: &oxikube_domain::Resource,
) -> Value {
    let conditions: Vec<Value> = PvcSummary::from_resource(patched)
        .map(|claim| {
            claim
                .conditions
                .iter()
                .map(|c| {
                    json!({
                        "type": &*c.kind,
                        "status": c.status.as_str(),
                        "message": c.message.as_deref(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "outcome": outcome,
        "from": from,
        "to": to.to_string(),
        "conditions": conditions,
    })
}
