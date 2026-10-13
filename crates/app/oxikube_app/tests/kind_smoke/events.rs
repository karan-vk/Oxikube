//! The `EventService` (E14-S02) against the real events adapter: one cluster-wide feed fills the
//! buffer with the events of a pod that starts (`Scheduled`, `Started`) and one that cannot be
//! scheduled (`FailedScheduling`, a `Warning`), indexed by the pod's uid and by namespace; the
//! overview's warnings read them; the feed stops when the last subscriber leaves.
//!
//! The pods live in the test's own `oxi-test-<rand>` namespace and every query is limited to it
//! (the feed itself is cluster-wide, as in the app, so other suites' events pass through the
//! buffer and are filtered out). The unschedulable pod asks for more CPU than any node has; it
//! is one pod, deleted with the namespace.

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::PostParams;
use oxikube_app::events::{EventFilter, EventService, EventsConfig, EventsState};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::store::Spawner;
use oxikube_app::{RecentWarnings, events::DEFAULT_EVENTS_BUFFER};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_ports::EventApi;
use oxikube_testkit::FakeClusterSourcePort;
use oxikube_testkit::integration::TestNamespace;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};
use crate::eventually;

fn tokio_spawner() -> Arc<dyn Spawner> {
    Arc::new(|task: BoxFuture<'static, ()>| {
        tokio::spawn(task);
    })
}

/// A pod no node can take: it asks for a thousand CPUs.
fn unschedulable(name: &str) -> Pod {
    let mut pod: serde_json::Value = oxikube_testkit::integration::pods::sleeper(name);
    pod["spec"]["containers"][0]["resources"] =
        serde_json::json!({ "requests": { "cpu": "1000" } });
    serde_json::from_value(pod).expect("pod json")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_feed_fills_the_buffer_and_its_indexes_from_the_cluster() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    let running: Pod =
        serde_json::from_value(oxikube_testkit::integration::pods::sleeper("ev-run"))
            .expect("pod json");
    let running = pods
        .create(&PostParams::default(), &running)
        .await
        .expect("create the sleeper");
    let stuck = pods
        .create(&PostParams::default(), &unschedulable("ev-stuck"))
        .await
        .expect("create the unschedulable pod");
    let (running_uid, stuck_uid) = (
        running.metadata.uid.expect("a uid"),
        stuck.metadata.uid.expect("a uid"),
    );

    let entry = catalog_entry(&kind.context);
    let cluster = entry.cluster.clone();
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
    sessions.connect(&cluster).await.expect("connect");

    let service = EventService::new(
        sessions.clone(),
        Arc::new(TokioClock),
        EventsConfig {
            idle_grace: Duration::ZERO,
            ..EventsConfig::default()
        },
    );
    service.start(tokio_spawner());
    let in_ns = EventFilter::new().namespaces([ns.name()]);
    let warnings = service.subscribe(&cluster, EventFilter::warnings().namespaces([ns.name()]));
    let diagnostics = || {
        format!(
            "state {:?}, status {:?}, held in the namespace: {:?}",
            service.state(&cluster),
            service.status(&cluster),
            service
                .query(&cluster, &in_ns)
                .iter()
                .map(|e| format!("{}/{} {}", e.regarding.name, e.reason, e.count))
                .collect::<Vec<_>>()
        )
    };

    eventually("the feed is ready", diagnostics, || async {
        service.state(&cluster) == EventsState::Ready
    })
    .await;
    let status = service.status(&cluster).expect("a running feed");
    assert_eq!(status.apis.len(), 1, "one API per cluster: {status:?}");
    assert!(matches!(
        status.apis[0],
        EventApi::EventsV1 | EventApi::Core
    ));
    assert_eq!(status.capacity, DEFAULT_EVENTS_BUFFER);

    // The running pod's events, through the involved-object index.
    let of_running = EventFilter::new().object(running_uid.as_str());
    eventually("the sleeper's Scheduled event", diagnostics, || async {
        service
            .query(&cluster, &of_running)
            .iter()
            .any(|e| &*e.reason == "Scheduled")
    })
    .await;
    // The stuck pod's warning, through the warnings index and the subscription's filter.
    eventually("FailedScheduling", diagnostics, || async {
        warnings.query().iter().any(|e| {
            &*e.reason == "FailedScheduling"
                && e.regarding_uid.as_deref() == Some(stuck_uid.as_str())
        })
    })
    .await;
    let (events, warning_count) = service
        .read(&cluster, |buffer| buffer.object_counts(&stuck_uid))
        .expect("a buffer");
    assert!(
        events >= 1 && warning_count >= 1,
        "{events} events, {warning_count} warnings"
    );
    assert_eq!(
        service.read(&cluster, |b| b.object_counts(&running_uid).1),
        Some(0),
        "the running pod has no warnings"
    );
    // Newest first, every one in the namespace.
    let all = service.query(&cluster, &in_ns);
    assert!(
        all.iter()
            .all(|e| e.namespace.as_deref() == Some(ns.name()))
    );
    assert!(
        all.windows(2).all(|w| w[0].last_seen >= w[1].last_seen),
        "{}",
        diagnostics()
    );
    // The overview's panel.
    let rows = RecentWarnings::latest(&service, &cluster, 50);
    assert!(
        rows.iter()
            .any(|r| r.object == "Pod/ev-stuck" && r.reason == "FailedScheduling"),
        "{rows:?}"
    );

    // The last subscriber leaves: the feed stops and the events are forgotten.
    drop(warnings);
    eventually("the feed stops", diagnostics, || async {
        service.state(&cluster) == EventsState::Idle
    })
    .await;
    assert!(service.query(&cluster, &in_ns).is_empty());
}
