//! Pod cells of the core catalogue: `Ready`, `Status`, `Restarts`, `IP` and `Last Restart`.
//!
//! The Pod kind's own provider ([`PodColumns`](crate::columns::PodColumns)) is the one the app
//! registers; these serve a table built on the core catalogue alone (a test harness, a provider
//! without kind specs). Both draw a cell with the same function ([`summary_cell`]), from the same
//! [`PodSummary`], so the two cannot drift; only the memo is the Pod provider's.

use jiff::Timestamp;
use oxikube_domain::{PodSummary, Resource};

use super::owner_cell;
use crate::columns::Cell;
use crate::columns::pods::{IP, LAST_RESTART, READY, RESTARTS, STATUS, summary_cell};

/// The `column` cell of a pod, as the Pod provider draws it (without its memo).
fn pod_cell<'a>(res: &'a Resource, column: &str, now: Timestamp) -> Cell<'a> {
    PodSummary::from_resource(res)
        .ok()
        .and_then(|s| summary_cell(column, res, &s, None, now))
        .unwrap_or_else(Cell::empty)
}

/// `READY`: `ready/total` containers, sorted by the ratio.
pub(crate) fn pod_ready<'a>(res: &'a Resource, now: Timestamp) -> Cell<'a> {
    pod_cell(res, READY, now)
}

/// `STATUS`: the kubectl status word, toned by meaning.
pub(crate) fn pod_status<'a>(res: &'a Resource, now: Timestamp) -> Cell<'a> {
    pod_cell(res, STATUS, now)
}

/// `RESTARTS`: the count, with how long ago the last one was (`3 (5m ago)`); sorted by count.
pub(crate) fn pod_restarts<'a>(res: &'a Resource, now: Timestamp) -> Cell<'a> {
    pod_cell(res, RESTARTS, now)
}

/// `IP`: the first of `status.podIPs`, else `status.podIP`.
pub(crate) fn pod_ip<'a>(res: &'a Resource, now: Timestamp) -> Cell<'a> {
    pod_cell(res, IP, now)
}

/// `LAST RESTART`: how long ago the most recent container restart finished.
pub(crate) fn pod_last_restart<'a>(res: &'a Resource, now: Timestamp) -> Cell<'a> {
    pod_cell(res, LAST_RESTART, now)
}

/// `CONTROLLED BY`: the controller owner, `Kind/name`.
pub(crate) fn controlled_by<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    res.meta
        .controller_ref()
        .map_or_else(Cell::empty, owner_cell)
}
