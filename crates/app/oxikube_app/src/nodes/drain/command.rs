//! The `node::Drain` handler.

use std::sync::Arc;

use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::drain::PodRef;
use serde_json::json;

use super::service::{NodeService, Requester};
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Registers `node::Drain` on `registry` over `service` (the binary installs it under
/// `oxikube_app::nodes`, next to cordon and uncordon), with its MCP tool stub
/// (`k8s.node_drain`, mutating).
///
/// The guard runs first: refused on a read-only cluster before anything is read, a
/// [`TypeName`](oxikube_domain::safety::ConfirmTier::TypeName) confirmation (risk high: the node's
/// name typed), and the drain's audit record with the flags; the handler adds the batch id and
/// the per-pod records ([`NodeService`]).
///
/// The output's `data` is `{"outcome": "drained" | "dry_run", "batch", "evicted": ["ns/name"],
/// "skipped": n}`.
///
/// # Errors
///
/// A [`RegisterError`] when the command is registered already.
pub fn register_drain_command(
    registry: &mut CommandRegistry,
    service: Arc<NodeService>,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::NODE_DRAIN)
        .ok_or(RegisterError::Undeclared(CommandId::NODE_DRAIN))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let service = service.clone();
        async move {
            let Command::NodeDrain { target, flags } = command else {
                return Err(OxiError::validation("not a node::Drain"));
            };
            let mutation = cx.require_mutation()?;
            let requester = Requester {
                who: Arc::from(cx.who()),
                initiator: cx.initiator(),
            };
            let report = service
                .drain(mutation, &requester, &target, &flags, |note| {
                    cx.note_audit(note)
                })
                .await?;
            let summary = &report.summary;
            let pods = |pods: &[PodRef]| pods.iter().map(ToString::to_string).collect::<Vec<_>>();
            let (message, outcome) = if summary.dry_run {
                (
                    format!(
                        "Dry run: draining node {} would evict {} pod(s)",
                        target.name,
                        summary.evicted.len()
                    ),
                    "dry_run",
                )
            } else {
                (
                    format!(
                        "Drained node {}: {} pod(s) evicted, {} skipped",
                        target.name,
                        summary.evicted.len(),
                        summary.skipped.len()
                    ),
                    "drained",
                )
            };
            Ok(CommandOutput {
                message: Some(message),
                data: Some(json!({
                    "outcome": outcome,
                    "batch": report.batch,
                    "evicted": pods(&summary.evicted),
                    "skipped": summary.skipped.len(),
                })),
            })
        }
    })
}
