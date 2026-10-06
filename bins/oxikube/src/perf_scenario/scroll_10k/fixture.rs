//! The fake feed generator behind `scroll-10k`: a cluster on testkit fakes whose pods watch lists
//! [`PODS`] pods and then delivers one [`churn`] batch per [`FRAME`] of its clock.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui::App;
use oxikube_app::{ClusterSession, ClusterSessionManager};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_ports::{ClusterContext, Delta, DeltaBatch, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort, Timeline, pod,
};
use oxikube_workspace::CommandDispatcher;

/// Pods listed.
pub const PODS: usize = 10_000;
/// One frame at 120 Hz.
pub const FRAME: Duration = Duration::from_micros(8_333);
/// Namespaces the pods spread over.
const NAMESPACES: usize = 8;
/// Churn batches scripted (more than any run draws).
const BATCHES: usize = super::FRAMES + 60;
/// Pods modified per batch.
const MODIFIED: usize = 6;
/// Pods deleted, and new pods created, per batch.
const REPLACED: usize = 2;
/// The first pod index a modify touches: the deletes (`REPLACED` a batch from index 0) never
/// reach it, so the pod count stays at [`PODS`].
const MODIFY_FROM: usize = 2_000;

/// Pod `i` at `version` (its restart count, so a modify changes a visible cell).
fn pod_at(i: usize, version: usize) -> Resource {
    let mut r = pod()
        .namespace(format!("oxikube-load-{}", i % NAMESPACES))
        .name(format!("load-{i:05}"))
        .restarts(u32::try_from(version).unwrap_or(u32::MAX))
        .build();
    r.meta.resource_version = Some(version.to_string().into());
    r
}

/// Batch `n` (from 1): [`MODIFIED`] modifies, [`REPLACED`] deletes and [`REPLACED`] new pods, ten
/// watch events. At 120 Hz that is 1 200 events/s, about twenty times what `cargo xtask load-pods
/// --count 10000 --churn` makes (1 % of the pods deleted and recreated every 5 s: about 300
/// events per 5 s with the delete's MODIFIED and DELETED and the create's ADDED).
pub fn churn(n: usize) -> DeltaBatch<Resource> {
    let span = PODS - MODIFY_FROM;
    let mut deltas = Vec::with_capacity(MODIFIED + 2 * REPLACED);
    for k in 0..MODIFIED {
        let i = MODIFY_FROM + (n * 97 + k * 1009) % span;
        deltas.push(Delta::Applied(pod_at(i, n + 1)));
    }
    for k in 0..REPLACED {
        deltas.push(Delta::Deleted(pod_at(n * REPLACED + k, 1)));
        deltas.push(Delta::Applied(pod_at(PODS + n * REPLACED + k, n + 1)));
    }
    DeltaBatch::from_deltas(deltas)
}

/// The pods kind as discovery serves it.
pub fn pods_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "Pod"),
        preferred: true,
        plural: "pods".into(),
        singular: "pod".into(),
        short_names: vec!["po".into()],
        categories: vec!["all".into()],
        verbs: VerbSet::from_names(["get", "list", "watch"]),
        namespaced: true,
    }
}

/// The table's commands go nowhere in the scenario.
pub struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

/// One connected cluster on fakes.
pub struct Fixture {
    pub sessions: ClusterSessionManager,
    pub cluster: ClusterId,
    ports: FakeClusterPorts,
}

impl Fixture {
    /// The cluster, connected, its pods watch scripted.
    pub fn connected() -> Result<Self> {
        let context = ContextName::new("perf");
        let cluster = ClusterId::new("/perf/kubeconfig", &context);
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let ports = connector.ports_for(&cluster);
        ports.discovery.set_kinds([pods_kind()]);
        let relist = DeltaBatch::from_deltas(vec![Delta::Restarted(
            (0..PODS).map(|i| pod_at(i, 1)).collect(),
        )]);
        let timeline = (1..=BATCHES).fold(
            Timeline::new().ok_at(Duration::ZERO, relist),
            |timeline, n| {
                let at = FRAME * u32::try_from(n).unwrap_or(u32::MAX);
                timeline.ok_at(at, churn(n))
            },
        );
        ports.resources.script().watch.push_ok(timeline.keep_open());
        let source = Arc::new(
            FakeClusterSourcePort::new().with_contexts([ClusterContext::new(
                cluster.clone(),
                context,
                SourceId("kubeconfig".into()),
            )]),
        );
        let sessions =
            ClusterSessionManager::new(connector, source, Arc::new(FakeClockPort::default()));
        futures::executor::block_on(sessions.connect(&cluster)).context("connecting")?;
        Ok(Self {
            sessions,
            cluster,
            ports,
        })
    }

    /// The connected session.
    pub fn session(&self) -> Result<ClusterSession> {
        self.sessions
            .get(&self.cluster)
            .context("the cluster has no session")
    }

    /// The clock the pods watch is replayed on: advancing it a [`FRAME`] delivers one batch.
    pub fn feed_clock(&self) -> Arc<FakeClockPort> {
        self.ports.resources.clock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn churn_keeps_the_pod_count_and_touches_ten_pods() {
        let mut live: std::collections::BTreeSet<String> =
            (0..PODS).map(|i| format!("load-{i:05}")).collect();
        for n in 1..=BATCHES {
            let batch = churn(n);
            assert_eq!(batch.deltas.len(), MODIFIED + 2 * REPLACED);
            for delta in batch.deltas {
                match delta {
                    Delta::Applied(r) => {
                        live.insert(r.meta.name.to_string());
                    }
                    Delta::Deleted(r) => {
                        assert!(live.remove(&*r.meta.name), "deleted a pod twice");
                    }
                    Delta::Restarted(_) => unreachable!(),
                }
            }
        }
        assert_eq!(live.len(), PODS);
    }
}
