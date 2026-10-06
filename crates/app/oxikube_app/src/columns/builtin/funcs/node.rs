//! Node cells, from [`NodeSummary`] (the kubectl node printer's `STATUS` and `ROLES`).

use jiff::Timestamp;
use oxikube_domain::{NodeSummary, Resource};

use super::{arr_at, spec, status_tone};
use crate::columns::{Cell, CellSort};

fn summary(res: &Resource) -> Option<NodeSummary> {
    NodeSummary::from_resource(res).ok()
}

/// `STATUS`: `Ready`, `NotReady` or `Unknown`, plus `,SchedulingDisabled` when cordoned.
pub(crate) fn node_status<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(s) = summary(res) else {
        return Cell::empty();
    };
    let tone = status_tone(&s.status);
    Cell::text(s.status.to_string()).with_tone(tone)
}

/// `ROLES`: comma-separated, `<none>` when the node has none (kubectl).
pub(crate) fn node_roles<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(s) = summary(res) else {
        return Cell::empty();
    };
    if s.roles.is_empty() {
        return Cell::text("<none>");
    }
    let roles: Vec<&str> = s.roles.iter().map(|r| &**r).collect();
    Cell::text(roles.join(","))
}

/// `VERSION`: the kubelet version.
pub(crate) fn node_version<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    summary(res)
        .and_then(|s| s.kubelet_version)
        .map_or_else(Cell::empty, |v| Cell::text(v.to_string()))
}

/// `INTERNAL-IP`.
pub(crate) fn node_internal_ip<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    address(res, "InternalIP")
}

/// `EXTERNAL-IP`.
pub(crate) fn node_external_ip<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    address(res, "ExternalIP")
}

fn address<'a>(res: &'a Resource, kind: &str) -> Cell<'a> {
    let addresses = arr_at(super::status(res), "addresses");
    addresses
        .iter()
        .find(|a| a.get("type").and_then(|t| t.as_str()) == Some(kind))
        .and_then(|a| a.get("address")?.as_str())
        .map_or_else(Cell::empty, Cell::text)
}

/// `TAINTS`: the number of taints, sorted numerically.
pub(crate) fn node_taints<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let n = arr_at(spec(res), "taints").len();
    Cell::shown(
        n.to_string(),
        CellSort::Int(i64::try_from(n).unwrap_or(i64::MAX)),
    )
}
