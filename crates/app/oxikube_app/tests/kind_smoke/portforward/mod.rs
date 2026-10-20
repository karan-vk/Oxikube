//! The port-forward manager end to end on kind (E15-S09): `PortForwardManager` over the real
//! session (`KubeConnector`, so `KubePortForward`), the shared `ResourceStores` and the
//! `SessionBackend`, no UI. Proves epic E15's done-when items 1 (a pod and a Service forward work
//! and survive pod replacement within 3 s) and 3 (favourites restart on launch).
//!
//! | Scenario | Test |
//! |---|---|
//! | A pod forward answers HTTP; B the pod is recreated and the forward recovers on the same port | `pod::a_pod_forward_answers_and_recovers_when_its_pod_is_recreated` |
//! | C a Service forward picks the EndpointSlice's ready pod; B the pod is replaced; scale to zero and back | `service::a_service_forward_follows_ready_pods_through_replacement_and_scale_to_zero` |
//! | D an auto-start favourite comes back after a restart over the same SQLite file | `favourites::an_auto_start_favourite_is_restored_after_a_restart` |
//! | E a closed remote port, a missing pod, a busy local port | `errors::closed_port_missing_pod_and_busy_local_port` |
//! | 50 concurrent forwards (perf budget smoke) | `concurrent::fifty_forwards_to_one_service` |
//!
//! Every object lives in the test's own `oxi-test-<rand>` namespace (the nginx fixtures of
//! `oxikube_testkit::integration::nginx`); nothing touches a node. Every wait is bounded. A
//! timing a test measures is printed as one `perf-jsonl {...}` line (numbers only).

mod concurrent;
mod errors;
mod favourites;
mod pod;
mod service;

use std::sync::Arc;
use std::time::{Duration, Instant};

use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{ListParams, PostParams};
use kube::{Api, Client};
use oxikube_app::portforward::{
    ForwardBackend, ForwardId, ForwardInfo, ForwardState, PortForwardManager, SessionBackend,
};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::store::{ResourceStores, Spawner, StoreRuntime};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::notification::ForwardPhase;
use oxikube_domain::portforward::{ForwardPort, ForwardSpec};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::{FakeClusterSourcePort, FakeNotificationSourcePort, SourceCall};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// Epic E15 done-when item 1: a forward reconnects within 3 s of its new pod becoming Ready.
pub const RECONNECT_TARGET: Duration = Duration::from_secs(3);

/// What a shared CI runner may add to [`RECONNECT_TARGET`] before a test fails. The measured
/// time is printed either way, so a slow trend shows long before this trips.
pub const CI_SLACK: Duration = Duration::from_secs(2);

/// One app's port-forward stack over the kind cluster: sessions, stores, the manager and the
/// notification hooks it reports to.
pub struct Forwarding {
    pub sessions: ClusterSessionManager,
    pub cluster: ClusterId,
    pub manager: PortForwardManager,
    pub notify: Arc<FakeNotificationSourcePort>,
    pub spawner: Arc<dyn Spawner>,
}

impl Forwarding {
    /// The stack with the kind context's session opened but not connected.
    pub fn open(kind: &Kind) -> Self {
        let entry = catalog_entry(&kind.context);
        let sessions = ClusterSessionManager::with_config(
            Arc::new(KubeConnector::new(
                kind.kubeconfig.clone(),
                PoolConfig::default(),
                ConnectorConfig::default(),
            )),
            Arc::new(FakeClusterSourcePort::new()),
            Arc::new(TokioClock),
            SessionManagerConfig::default(),
        );
        sessions.open(&entry, SessionOptions::default());
        let spawner: Arc<dyn Spawner> = Arc::new(|task| {
            tokio::spawn(task);
        });
        let stores = Arc::new(ResourceStores::new(StoreRuntime {
            spawner: spawner.clone(),
            clock: Arc::new(TokioClock),
            probe: None,
        }));
        let backend: Arc<dyn ForwardBackend> =
            Arc::new(SessionBackend::new(sessions.clone(), stores));
        let notify = Arc::new(FakeNotificationSourcePort::new());
        let manager = PortForwardManager::new(
            backend,
            spawner.clone(),
            Arc::new(TokioClock),
            Some(notify.clone()),
        );
        Self {
            sessions,
            cluster: entry.cluster,
            manager,
            notify,
            spawner,
        }
    }

    /// The stack, connected.
    pub async fn connect(kind: &Kind) -> Self {
        let forwarding = Self::open(kind);
        forwarding
            .sessions
            .connect(&forwarding.cluster)
            .await
            .expect("connect to kind");
        forwarding
    }

    /// A forward of `remote` on the `kind` (`Pod` or `Service`) `name` in `namespace`, on an
    /// automatic local port.
    pub fn spec(
        &self,
        kind: &str,
        namespace: &str,
        name: &str,
        remote: ForwardPort,
    ) -> ForwardSpec {
        ForwardSpec::new(
            ResourceRef::namespaced(
                self.cluster.clone(),
                Gvk::new("", "v1", kind),
                namespace,
                name,
            ),
            remote,
        )
    }

    /// Waits (bounded by [`crate::DEADLINE`]) until forward `id` satisfies `done`.
    pub async fn wait(
        &self,
        id: ForwardId,
        what: &str,
        done: impl Fn(&ForwardInfo) -> bool,
    ) -> ForwardInfo {
        let mut changes = self.manager.subscribe();
        let deadline = tokio::time::Instant::now() + crate::DEADLINE;
        loop {
            let info = self.manager.get(id).expect("the forward is listed");
            if done(&info) {
                return info;
            }
            // A change wakes at once; the timeout only re-reads in case one was coalesced away.
            let _ = tokio::time::timeout(Duration::from_millis(200), changes.changed()).await;
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for forward {id} to be {what}; it is {:?}",
                self.manager.get(id).map(|i| i.state)
            );
        }
    }

    /// The phases forward `id` reported to the notification hooks, as `(from, to)`, in order.
    pub fn transitions(&self, id: ForwardId) -> Vec<(ForwardPhase, ForwardPhase)> {
        let id = id.to_string();
        self.notify
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                SourceCall::PortForward(change) if change.forward_id == id => {
                    Some((change.from, change.to))
                }
                _ => None,
            })
            .collect()
    }
}

/// Whether `info` is `Active`.
pub fn is_active(info: &ForwardInfo) -> bool {
    matches!(info.state, ForwardState::Active { .. })
}

/// The pod `info` is active on.
pub fn active_pod(info: &ForwardInfo) -> Option<&str> {
    match &info.state {
        ForwardState::Active { pod, .. } => Some(pod),
        _ => None,
    }
}

/// Creates the object `manifest` (a testkit fixture) as a `K` in `namespace`.
pub async fn create<K>(client: &Client, namespace: &str, manifest: Value)
where
    K: kube::Resource<Scope = NamespaceResourceScope>
        + Clone
        + DeserializeOwned
        + Serialize
        + std::fmt::Debug,
    K::DynamicType: Default,
{
    let object: K = serde_json::from_value(manifest).expect("a valid fixture");
    Api::<K>::namespaced(client.clone(), namespace)
        .create(&PostParams::default(), &object)
        .await
        .expect("create the fixture");
}

/// A pod by name and uid (a StatefulSet's replacement has the same name and another uid).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodId {
    pub name: String,
    pub uid: String,
}

/// The pods labelled `app=<app>` that are Ready and not terminating, by name.
pub async fn ready_pods(client: &Client, namespace: &str, app: &str) -> Vec<PodId> {
    let list = Api::<Pod>::namespaced(client.clone(), namespace)
        .list(&ListParams::default().labels(&format!("app={app}")))
        .await
        .map(|l| l.items)
        .unwrap_or_default();
    let ready = |pod: &Pod| {
        pod.metadata.deletion_timestamp.is_none()
            && pod
                .status
                .as_ref()
                .and_then(|s| s.conditions.as_ref())
                .is_some_and(|c| c.iter().any(|c| c.type_ == "Ready" && c.status == "True"))
    };
    let mut pods: Vec<PodId> = list
        .into_iter()
        .filter(|p| ready(p))
        .filter_map(|p| {
            Some(PodId {
                name: p.metadata.name?,
                uid: p.metadata.uid?,
            })
        })
        .collect();
    pods.sort_by(|a, b| a.name.cmp(&b.name));
    pods
}

/// Waits until a pod labelled `app=<app>` is Ready; returns it. An image pull is not expected
/// (kind-up pulled nginx), so the deadline covers the whole start.
pub async fn wait_ready_pod(client: &Client, namespace: &str, app: &str) -> PodId {
    let started = Instant::now();
    loop {
        if let Some(pod) = ready_pods(client, namespace, app).await.into_iter().next() {
            return pod;
        }
        assert!(
            started.elapsed() < crate::DEADLINE * 2,
            "no ready pod app={app} in {namespace}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A recovery timed against the pod: when the replacement became Ready (seen by polling the API
/// server) and when the manager went `Active` on it again.
#[derive(Debug)]
pub struct Recovery {
    /// The pod the forward is active on now.
    pub pod: String,
    /// From the replacement's readiness to `Active` (zero when the manager was first).
    pub reconnect: Duration,
    /// From the delete (or scale) to `Active`.
    pub total: Duration,
}

/// After `old`, the pod behind forward `id`, went away at `since`: polls the API server for a
/// ready replacement (another uid) and the manager for `Active` on a pod other than `old` (by
/// name, or after the forward was seen leaving `Active` when the replacement keeps the name),
/// every 25 ms, and times both. Start it right after the delete so no readiness is missed.
pub async fn time_recovery(
    forwarding: &Forwarding,
    client: &Client,
    namespace: &str,
    app: &str,
    id: ForwardId,
    since: Instant,
    old: &PodId,
) -> Recovery {
    let mut ready_at = None;
    let mut dropped = false;
    loop {
        let now = Instant::now();
        if ready_at.is_none()
            && ready_pods(client, namespace, app)
                .await
                .iter()
                .any(|p| p.uid != old.uid)
        {
            ready_at = Some(now);
        }
        let info = forwarding.manager.get(id).expect("listed");
        match active_pod(&info) {
            Some(pod) if pod != old.name || dropped => {
                let active_at = Instant::now();
                return Recovery {
                    pod: pod.to_owned(),
                    reconnect: active_at.saturating_duration_since(ready_at.unwrap_or(active_at)),
                    total: active_at - since,
                };
            }
            Some(_) => {}
            None => dropped = true,
        }
        assert!(
            since.elapsed() < crate::DEADLINE * 2,
            "forward {id} did not recover: {:?}",
            info.state
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Prints a measurement as one machine-readable line (numbers only), the format the integration
/// workflow extracts into `perf.jsonl`.
pub fn perf_line(scenario: &str, fields: &[(&str, Duration)]) {
    let mut record = serde_json::Map::new();
    record.insert("scenario".into(), scenario.into());
    for (name, value) in fields {
        let millis = u64::try_from(value.as_millis()).unwrap_or(u64::MAX);
        record.insert(format!("{name}_ms"), millis.into());
    }
    eprintln!("perf-jsonl {}", Value::Object(record));
}

/// Asserts `recovery` meets the reconnect target (with [`CI_SLACK`]).
pub fn assert_reconnect(scenario: &str, recovery: &Recovery) {
    perf_line(
        scenario,
        &[("reconnect", recovery.reconnect), ("total", recovery.total)],
    );
    assert!(
        recovery.reconnect <= RECONNECT_TARGET + CI_SLACK,
        "{scenario}: reconnect took {:?} after the new pod was Ready (target {RECONNECT_TARGET:?})",
        recovery.reconnect
    );
}
