//! The Pod rule of [`health_of`](super::health_of).
//!
//! It reads the same inputs [`PodSummary`](super::PodSummary) derives `READY` and `STATUS` from
//! (phase, `deletionTimestamp`, container statuses) but borrows from the JSON and allocates
//! nothing, because the overview tallies rate every pod of the cache.

use crate::json::JsonRef;

use super::pod::{StateView, is_restartable};
use super::{PodPhase, arr_of, bool_of, i32_of, str_of, sub};
use crate::resource::Resource;

/// A Pod is healthy when its phase is `Succeeded`, or it is `Running`, not being deleted, and
/// every container that has not completed (exited 0) is ready.
///
/// The containers are the spec's regular containers plus its sidecars (init containers with
/// `restartPolicy: Always`); a container with no status yet is not ready. Plain init containers
/// and ephemeral (debug) containers never count, as in the `READY` column.
pub(super) fn pod_is_healthy(res: &Resource) -> bool {
    let status = sub(res.json(), "status");
    match PodPhase::parse(str_of(status, "phase")) {
        PodPhase::Succeeded => true,
        PodPhase::Running => {
            res.meta.deletion.is_none() && containers_ready(sub(res.json(), "spec"), status)
        }
        _ => false,
    }
}

fn containers_ready(spec: JsonRef<'_>, status: JsonRef<'_>) -> bool {
    let statuses = arr_of(status, "containerStatuses");
    if statuses.len() < arr_of(spec, "containers").len() || !statuses.iter().all(ready_or_completed)
    {
        return false;
    }
    let sidecar_statuses = arr_of(status, "initContainerStatuses");
    arr_of(spec, "initContainers")
        .iter()
        .filter(|&c| is_restartable(c))
        .all(|sidecar| {
            let name = str_of(sidecar, "name");
            sidecar_statuses
                .iter()
                .find(|&s| str_of(s, "name") == name)
                .is_some_and(ready_or_completed)
        })
}

/// A container status that is ready, or terminated with exit code 0.
fn ready_or_completed(cs: JsonRef<'_>) -> bool {
    bool_of(cs, "ready")
        || StateView::of(cs)
            .terminated
            .is_some_and(|t| i32_of(t, "exitCode") == 0)
}
