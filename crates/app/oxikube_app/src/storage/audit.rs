//! What the audit record of a storage command says that its payload alone cannot.

use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::view::storage::PvcSummary;

use super::read::claim_gvk;
use crate::session::ClusterSessionManager;

/// The audit detail of a `pvc::Resize` that does not say which size it grows from (an MCP or
/// scripted caller may omit `from`): `from=<live size> to=<size>`, the live size read from the
/// claim as the handler reads it. `None` for any other command, for a resize that names its old
/// size (the [policy detail](crate::guard::audit_detail) has it), and when the claim cannot be
/// read (the record then says `to=<size>` only, and the handler reports the failure).
pub(crate) async fn resolved_audit_detail(
    command: &Command,
    cluster: &ClusterId,
    sessions: &ClusterSessionManager,
) -> Option<String> {
    let Command::PvcResize { target, size, from } = command else {
        return None;
    };
    if from.as_deref().is_some_and(|from| !from.trim().is_empty()) {
        return None;
    }
    let reader = sessions.get(cluster)?.resources()?;
    let live = reader
        .get(&claim_gvk(), target.namespace(), &target.name)
        .await
        .ok()?;
    let current = PvcSummary::from_resource(&live).ok()?.current_request()?;
    Some(format!("from={current} to={size}"))
}
