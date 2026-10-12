//! The cells of a pod row, read from its memoised [`PodSummary`] and, for usage, from the
//! metrics source. The core catalogue's pod cells draw through [`cell`] too, so a pod cell has
//! one implementation.

use std::borrow::Cow;

use jiff::Timestamp;
use oxikube_domain::view::PodPhase;
use oxikube_domain::{Age, PodSummary, Quantity, Resource};

use super::defs::{
    CONTROLLED_BY, CPU, CPU_OF_LIMIT, CPU_OF_REQUEST, IP, LAST_RESTART, MEMORY, MEMORY_OF_LIMIT,
    MEMORY_OF_REQUEST, NODE, NOMINATED_NODE, QOS, READY, RESTARTS, SERVICE_ACCOUNT, STATUS,
};
use crate::columns::builtin::{Metric, MetricsSource, owner_cell, ready_tone, status_tone};
use crate::columns::{Cell, CellSort, Tone};

/// What a usage cell shows without a sample, and a `%` cell without a sample, request or limit.
pub const ABSENT: &str = "–";

/// The cell shown when a value is absent: `–`, sorted last.
pub(super) fn absent<'a>() -> Cell<'a> {
    Cell::shown(ABSENT, CellSort::None)
}

/// The cell of a computed pod column (`None`: not one of them).
pub(crate) fn cell<'a>(
    column: &str,
    pod: &'a Resource,
    s: &PodSummary,
    metrics: Option<&dyn MetricsSource>,
    now: Timestamp,
) -> Option<Cell<'a>> {
    let owned = |v: Option<&std::sync::Arc<str>>| {
        v.map_or_else(Cell::empty, |v| Cell::text(Cow::Owned(v.to_string())))
    };
    Some(match column {
        READY => ready(s),
        STATUS => Cell::text(Cow::Owned(s.status.to_string())).with_tone(status_tone(&s.status)),
        RESTARTS => Cell::shown(
            s.restarts_display(now),
            CellSort::Int(i64::from(s.restarts)),
        ),
        LAST_RESTART => s
            .last_restart
            .map_or_else(Cell::empty, |at| Cell::age(Age::between(at, now))),
        CPU => usage(pod, Metric::Cpu, metrics, now),
        MEMORY => usage(pod, Metric::Memory, metrics, now),
        CPU_OF_REQUEST => percent(pod, s, Metric::Cpu, false, metrics, now),
        CPU_OF_LIMIT => percent(pod, s, Metric::Cpu, true, metrics, now),
        MEMORY_OF_REQUEST => percent(pod, s, Metric::Memory, false, metrics, now),
        MEMORY_OF_LIMIT => percent(pod, s, Metric::Memory, true, metrics, now),
        IP => owned(s.ip.as_ref()),
        NODE => owned(s.node.as_ref()),
        NOMINATED_NODE => owned(s.nominated_node.as_ref()),
        QOS => s.qos.map_or_else(Cell::empty, |q| Cell::text(q.as_str())),
        SERVICE_ACCOUNT => owned(s.service_account.as_ref()),
        CONTROLLED_BY => s
            .controlled_by
            .as_ref()
            .map_or_else(Cell::empty, owner_cell),
        _ => return None,
    })
}

/// `READY`: `ready/total`, sorted by the ratio; amber while short (neutral once succeeded).
fn ready<'a>(s: &PodSummary) -> Cell<'a> {
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

/// The usage of `metric` as the source reports it, else `–`.
fn usage<'a>(
    pod: &Resource,
    metric: Metric,
    metrics: Option<&dyn MetricsSource>,
    now: Timestamp,
) -> Cell<'a> {
    metrics
        .and_then(|m| m.cell(pod, metric, now))
        .filter(|c| !c.is_blank())
        .unwrap_or_else(absent)
}

/// The usage of `metric` as an exact quantity: the source's cell sort key.
fn usage_value(
    pod: &Resource,
    metric: Metric,
    metrics: Option<&dyn MetricsSource>,
    now: Timestamp,
) -> Option<Quantity> {
    match metrics?.cell(pod, metric, now)?.sort() {
        CellSort::Quantity(q) => Some(q),
        _ => None,
    }
}

/// Usage over `of` (the pod's request or limit) in whole percent, k9s' `%CPU/R` style: `–`
/// without a sample or without the request or limit. Over a limit, amber and red from the
/// source's [`UsageThresholds`](crate::columns::UsageThresholds) (k9s' 70 % and 90 % by
/// default); over a request, neutral.
fn percent<'a>(
    pod: &Resource,
    summary: &PodSummary,
    metric: Metric,
    limit: bool,
    metrics: Option<&dyn MetricsSource>,
    now: Timestamp,
) -> Cell<'a> {
    let amounts = if limit {
        summary.resources.limits
    } else {
        summary.resources.requests
    };
    let of = match metric {
        Metric::Cpu => amounts.cpu,
        Metric::Memory => amounts.memory,
    };
    let Some(pct) = of.and_then(|total| usage_value(pod, metric, metrics, now)?.percent_of(&total))
    else {
        return absent();
    };
    let tone = match metrics {
        Some(source) if limit => source.thresholds().of(metric == Metric::Cpu).tone(pct),
        _ => Tone::Neutral,
    };
    Cell::float(format!("{pct:.0}"), pct).with_tone(tone)
}
