//! The poll loop of one watched cluster: poll each watched kind when its [`Cadence`] says it is
//! due, record the answer, sleep until the next one is due.
//!
//! The loop holds the service weakly and reads the entry afresh each round, so a change of the
//! watched kinds, the interval or the namespace selection is seen on the next round; the service
//! restarts the loop (dropping its task) when such a change must take effect at once. Sleeping is
//! on the service's [`ClockPort`](oxikube_ports::ClockPort), so tests drive it with a fake clock.

use std::sync::{Arc, Weak};

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::MetricsOutcome;

#[cfg(doc)]
use super::cadence::Cadence;
use super::service::Inner;
use super::state::MetricsKind;

/// What one round decided.
enum Round {
    /// Poll these kinds now.
    Poll(Vec<MetricsKind>),
    /// Nothing is due before this.
    Sleep(jiff::Timestamp),
    /// The cluster is gone from the service, or nothing is watched.
    Stop,
}

/// The loop of `cluster`, until its task is dropped or the service goes.
pub(super) async fn poll_loop(inner: Weak<Inner>, cluster: ClusterId) {
    loop {
        let (round, clock) = {
            let Some(inner) = inner.upgrade() else {
                return;
            };
            (plan(&inner, &cluster), inner.clock.clone())
        };
        match round {
            Round::Stop => return,
            Round::Sleep(until) => clock.sleep_until(until).await,
            Round::Poll(kinds) => {
                for kind in kinds {
                    if !poll(&inner, &cluster, kind).await {
                        return;
                    }
                }
            }
        }
    }
}

/// The kinds due now, else when the next one is.
fn plan(inner: &Inner, cluster: &ClusterId) -> Round {
    let now = inner.clock.now();
    let state = inner.state.lock();
    let Some(entry) = state.clusters.get(cluster) else {
        return Round::Stop;
    };
    let base = entry.config.poll_interval;
    let mut due = Vec::new();
    let mut next: Option<jiff::Timestamp> = None;
    for kind in entry.wanted() {
        let cadence = &entry.cadence[kind.index()];
        if cadence.is_due(base, now) {
            due.push(kind);
        } else if let Some(at) = cadence.due(base) {
            next = Some(next.map_or(at, |n| n.min(at)));
        }
    }
    match (due.is_empty(), next) {
        (false, _) => Round::Poll(due),
        (true, Some(at)) => Round::Sleep(at),
        (true, None) => Round::Stop,
    }
}

/// One call for `kind`, its answer recorded. `false` when the service or the session is gone.
async fn poll(inner: &Weak<Inner>, cluster: &ClusterId, kind: MetricsKind) -> bool {
    let (port, namespaces) = {
        let Some(inner) = inner.upgrade() else {
            return false;
        };
        let Some(session) = inner.sessions.get(cluster) else {
            return false;
        };
        let Some(port) = session.metrics() else {
            return false;
        };
        if let Some(entry) = inner.state.lock().clusters.get(cluster) {
            entry.usage.set_loading(kind);
        }
        (port, session.namespace_selection().clone())
    };
    let result: OxiResult<MetricsOutcome> = match kind {
        MetricsKind::Nodes => port.node_metrics(cluster).await,
        MetricsKind::Pods => port.pod_metrics(cluster, &namespaces).await,
    };
    let Some(inner) = inner.upgrade() else {
        return false;
    };
    record(&inner, cluster, kind, &result);
    true
}

/// Records `result` in the cache and the cadence, and tells the subscribers.
fn record(
    inner: &Arc<Inner>,
    cluster: &ClusterId,
    kind: MetricsKind,
    result: &OxiResult<MetricsOutcome>,
) {
    let at = inner.clock.now();
    let usage = {
        let mut state = inner.state.lock();
        let Some(entry) = state.clusters.get_mut(cluster) else {
            return;
        };
        entry.cadence[kind.index()].record(at, result);
        entry.usage.clone()
    };
    match result {
        Ok(MetricsOutcome::Available(samples)) => {
            tracing::debug!(%cluster, ?kind, samples = samples.len(), "metrics poll answered");
        }
        Ok(MetricsOutcome::Unavailable(reason)) => {
            tracing::debug!(%cluster, ?kind, ?reason, "metrics unavailable");
        }
        Err(error) => tracing::debug!(%cluster, ?kind, %error, "metrics poll failed"),
    }
    usage.apply(kind, at, result);
    inner.notify(cluster);
}
