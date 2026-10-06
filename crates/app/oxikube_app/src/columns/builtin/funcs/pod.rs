//! Pod cells. `Ready`, `Status` and `Restarts` come from [`PodSummary`], which follows the
//! kubectl pod printer (init containers, waiting reasons, `Terminating`, `NodeLost`).

use std::borrow::Cow;

use jiff::Timestamp;
use oxikube_domain::view::PodPhase;
use oxikube_domain::{Age, PodSummary, Resource};

use super::{ready_tone, status, status_tone};
use crate::columns::{Cell, CellSort, Tone};

fn summary(res: &Resource) -> Option<PodSummary> {
    PodSummary::from_resource(res).ok()
}

/// `READY`: `ready/total` containers, sorted by the ratio.
pub(crate) fn pod_ready<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(s) = summary(res) else {
        return Cell::empty();
    };
    let ratio = if s.total == 0 {
        0.0
    } else {
        f64::from(s.ready) / f64::from(s.total)
    };
    let tone = if s.phase == PodPhase::Succeeded {
        Tone::Neutral
    } else {
        ready_tone(s.ready, s.total)
    };
    Cell::float(s.ready_display(), ratio).with_tone(tone)
}

/// `STATUS`: the kubectl status word, toned by meaning.
pub(crate) fn pod_status<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(s) = summary(res) else {
        return Cell::empty();
    };
    let tone = status_tone(&s.status);
    Cell::text(s.status.to_string()).with_tone(tone)
}

/// `RESTARTS`: the count, with how long ago the last one was (`3 (5m ago)`); sorted by count.
pub(crate) fn pod_restarts<'a>(res: &'a Resource, now: Timestamp) -> Cell<'a> {
    let Some(s) = summary(res) else {
        return Cell::empty();
    };
    Cell::shown(
        s.restarts_display(now),
        CellSort::Int(i64::from(s.restarts)),
    )
}

/// `IP`: the first of `status.podIPs`, else `status.podIP`.
pub(crate) fn pod_ip<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let status = status(res);
    let from_list = status
        .get("podIPs")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|p| p.get("ip"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    from_list
        .or_else(|| status.get("podIP").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map_or_else(Cell::empty, Cell::text)
}

/// `LAST RESTART`: how long ago the most recent container restart finished.
pub(crate) fn pod_last_restart<'a>(res: &'a Resource, now: Timestamp) -> Cell<'a> {
    summary(res)
        .and_then(|s| s.last_restart)
        .map_or_else(Cell::empty, |at| Cell::age(Age::between(at, now)))
}

/// `CONTROLLED BY`: the controller owner, `Kind/name`.
pub(crate) fn controlled_by<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    res.meta.controller_ref().map_or_else(Cell::empty, |owner| {
        Cell::text(Cow::Owned(format!("{}/{}", owner.kind, owner.name)))
    })
}
