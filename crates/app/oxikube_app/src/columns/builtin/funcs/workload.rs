//! Cells shared by the replica-based workloads (Deployment, ReplicaSet, StatefulSet,
//! DaemonSet), from [`WorkloadSummary`], and by ReplicationControllers.

use std::borrow::Cow;

use jiff::Timestamp;
use oxikube_domain::json::JsonRef;
use oxikube_domain::{Resource, WorkloadSummary};

use super::{images_text, ready_tone, selector_text};
use crate::columns::{Cell, CellSort};

fn summary(res: &Resource) -> Option<WorkloadSummary> {
    WorkloadSummary::from_resource(res).ok()
}

fn count<'a>(res: &Resource, pick: impl Fn(&WorkloadSummary) -> u32) -> Cell<'a> {
    summary(res).map_or_else(Cell::empty, |s| Cell::int(i64::from(pick(&s))))
}

/// `READY`: `ready/desired`, sorted by the ratio; amber while short.
pub(crate) fn workload_ready<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(s) = summary(res) else {
        return Cell::empty();
    };
    let ratio = if s.desired == 0 {
        1.0
    } else {
        f64::from(s.ready) / f64::from(s.desired)
    };
    Cell::float(s.ready_display(), ratio).with_tone(ready_tone(s.ready, s.desired))
}

/// `DESIRED`.
pub(crate) fn workload_desired<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    count(res, |s| s.desired)
}

/// `CURRENT`.
pub(crate) fn workload_current<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    count(res, |s| s.current)
}

/// The number of ready replicas (`READY` of a ReplicaSet or DaemonSet).
pub(crate) fn workload_ready_count<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    count(res, |s| s.ready)
}

/// `UP-TO-DATE`.
pub(crate) fn workload_updated<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    count(res, |s| s.updated)
}

/// `AVAILABLE`.
pub(crate) fn workload_available<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    count(res, |s| s.available)
}

/// `IMAGES` of the pod template, comma-separated.
pub(crate) fn workload_images<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let spec = res
        .json()
        .pointer("/spec/template/spec")
        .unwrap_or(JsonRef::NULL);
    Cell::text(images_text(spec))
}

/// `SELECTOR`: the pod selector's `matchLabels`.
pub(crate) fn workload_selector<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    Cell::text(selector_text(
        res.json().pointer("/spec/selector/matchLabels"),
    ))
}

/// `NODE SELECTOR` of a DaemonSet's pod template.
pub(crate) fn node_selector<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    Cell::text(selector_text(
        res.json().pointer("/spec/template/spec/nodeSelector"),
    ))
}

/// ReplicationController `DESIRED`: `spec.replicas`, default 1.
pub(crate) fn rc_desired<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let n = res
        .json()
        .pointer("/spec/replicas")
        .and_then(JsonRef::as_i64);
    Cell::int(n.unwrap_or(1))
}

/// ReplicationController `CURRENT`: `status.replicas`.
pub(crate) fn rc_current<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    Cell::int(
        res.json()
            .pointer("/status/replicas")
            .and_then(JsonRef::as_i64)
            .unwrap_or(0),
    )
}

/// ReplicationController `READY`: `status.readyReplicas`.
pub(crate) fn rc_ready<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    Cell::int(
        res.json()
            .pointer("/status/readyReplicas")
            .and_then(JsonRef::as_i64)
            .unwrap_or(0),
    )
}

/// ReplicationController `SELECTOR`: `spec.selector` is a plain label map.
pub(crate) fn rc_selector<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    Cell::text(Cow::Owned(selector_text(
        res.json().pointer("/spec/selector"),
    )))
}

/// A cell from a pair, used where a count needs a text form that differs from its number.
pub(crate) fn counted<'a>(text: String, n: u32) -> Cell<'a> {
    Cell::shown(text, CellSort::Int(i64::from(n)))
}
