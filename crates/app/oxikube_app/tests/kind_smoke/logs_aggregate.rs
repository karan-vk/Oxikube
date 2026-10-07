//! Multi-pod logs (E08-S04) over the real adapters: a Deployment of three busybox pods that echo
//! their own name and a counter, read through `LogService::open_aggregate` on the Tokio runtime,
//! the wall clock, the real `LogPort` and the real resource reader (the pod watch).
//!
//! * the selector of the Deployment resolves to its pods and every pod is streamed;
//! * the merged buffer is ordered by the kubelet's timestamps, and each line is attributed to the
//!   pod that wrote it;
//! * a pod that joins afterwards is announced (`PodChange::Added`) and streamed;
//! * dropping the session cancels the watch and the streams.
//!
//! Everything lives in the test's own `oxi-test-<rand>` namespace.

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::{ListParams, PostParams};
use oxikube_app::logs::{
    AggregatePorts, AggregateSpec, LogConfig, LogRuntime, LogService, PodChange,
};
use oxikube_app::store::Spawner;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_kube::{KubeDiscovery, KubeLogs, KubeResources};
use oxikube_ports::LogOptions;
use oxikube_testkit::images::BUSYBOX;
use oxikube_testkit::integration::TestNamespace;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};
use crate::eventually;

const ECHO: &str = "i=0; while true; do echo \"$HOSTNAME $i\"; i=$((i+1)); sleep 0.2; done";

fn deployment() -> Deployment {
    serde_json::from_value(serde_json::json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": { "name": "web" },
        "spec": {
            "replicas": 3,
            "selector": { "matchLabels": { "app": "web" } },
            "template": {
                "metadata": { "labels": { "app": "web" } },
                "spec": {
                    "terminationGracePeriodSeconds": 0,
                    "containers": [{
                        "name": "main",
                        "image": BUSYBOX,
                        "command": ["sh", "-c", ECHO],
                    }],
                },
            },
        },
    }))
    .expect("a deployment")
}

fn late_pod() -> Pod {
    serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": "web-late", "labels": { "app": "web" } },
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": [{ "name": "main", "image": BUSYBOX, "command": ["sh", "-c", ECHO] }],
        },
    }))
    .expect("a pod")
}

fn service() -> LogService {
    let spawner: Arc<dyn Spawner> = Arc::new(|task: BoxFuture<'static, ()>| {
        tokio::spawn(task);
    });
    LogService::new(
        LogRuntime {
            spawner,
            clock: Arc::new(TokioClock),
        },
        LogConfig::default(),
    )
}

async fn running_pods(pods: &Api<Pod>) -> usize {
    pods.list(&ListParams::default().labels("app=web"))
        .await
        .map_or(0, |list| {
            list.items
                .iter()
                .filter(|p| {
                    p.status
                        .as_ref()
                        .and_then(|s| s.phase.as_deref())
                        .is_some_and(|phase| phase == "Running")
                })
                .count()
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deployments_pods_are_merged_in_timestamp_order_and_a_new_pod_is_announced() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    Api::<Deployment>::namespaced(client.clone(), ns.name())
        .create(&PostParams::default(), &deployment())
        .await
        .expect("create the deployment");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    eventually(
        "three running pods",
        || String::new(),
        || async { running_pods(&pods).await == 3 },
    )
    .await;

    let resources = KubeResources::new(client.clone(), KubeDiscovery::new(client.clone()));
    let ports = AggregatePorts {
        logs: Arc::new(KubeLogs::new(client.clone())),
        resources: Arc::new(resources),
    };
    let cluster: ClusterId = catalog_entry(&kind.context).cluster;
    let target = ResourceRef::namespaced(
        cluster,
        Gvk::new("apps", "v1", "Deployment"),
        ns.name(),
        "web",
    );
    let service = service();
    let session = service.open_aggregate(
        ports,
        AggregateSpec::of(&target).expect("a deployment"),
        LogOptions::follow(),
    );
    let view = session.aggregate().clone();
    eventually(
        "lines of all three pods",
        || {
            format!(
                "{:?} with {} lines, {:?}",
                session.state(),
                session.len(),
                view.sources()
            )
        },
        || async {
            let pods_seen: std::collections::HashSet<String> =
                session.read(|b, _| b.iter().map(|e| e.pod.to_string()).collect());
            pods_seen.len() == 3 && session.len() >= 30
        },
    )
    .await;

    assert_eq!(view.selector().as_deref(), Some("app=web"));
    assert_eq!(view.sources().len(), 3, "one stream per pod");
    session.read(|buffer, _| {
        // Every line says which pod wrote it: the echo starts with the hostname, the pod's name.
        for entry in buffer.iter() {
            let host = entry.text.split(' ').next().unwrap_or_default();
            assert_eq!(
                host, &*entry.pod,
                "line {:?} of pod {}",
                entry.text, entry.pod
            );
        }
        // Merged by the kubelet's timestamps. The reorder window is best effort at the edges
        // (a stream that opens late), so allow a sliver of inversions, never a disorder.
        let lines: Vec<_> = buffer.iter().collect();
        let inversions = lines.windows(2).filter(|w| w[1].ts < w[0].ts).count();
        assert!(
            inversions * 50 <= lines.len(),
            "{inversions} of {} lines out of timestamp order",
            lines.len()
        );
        // Each pod's own lines keep their order exactly.
        for pod in lines
            .iter()
            .map(|e| e.pod.clone())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let numbers: Vec<u64> = lines
                .iter()
                .filter(|e| e.pod == pod)
                .filter_map(|e| e.text.rsplit(' ').next()?.parse().ok())
                .collect();
            assert!(
                numbers.windows(2).all(|w| w[1] > w[0]),
                "{pod}: {numbers:?}"
            );
        }
    });

    // A pod that joins the selector afterwards is announced and streamed.
    pods.create(&PostParams::default(), &late_pod())
        .await
        .expect("create the late pod");
    eventually(
        "the late pod announced and read",
        || format!("{:?}", view.events_after(None)),
        || async {
            let announced = view
                .events_after(None)
                .iter()
                .any(|e| &*e.pod == "web-late" && e.change == PodChange::Added);
            let read = session.read(|b, _| b.iter().any(|e| &*e.pod == "web-late"));
            announced && read
        },
    )
    .await;

    // Cancel on drop: the session ends and the service lists nothing.
    let reader = session.reader();
    drop(session);
    assert!(service.sessions().is_empty());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(reader.state().is_terminal());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_deployment_that_does_not_exist_is_a_failed_session() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let ports = AggregatePorts {
        logs: Arc::new(KubeLogs::new(client.clone())),
        resources: Arc::new(KubeResources::new(
            client.clone(),
            KubeDiscovery::new(client),
        )),
    };
    let target = ResourceRef::namespaced(
        catalog_entry(&kind.context).cluster,
        Gvk::new("apps", "v1", "Deployment"),
        ns.name(),
        "nope",
    );
    let session = service().open_aggregate(
        ports,
        AggregateSpec::of(&target).expect("a deployment"),
        LogOptions::follow(),
    );
    eventually(
        "a failed session",
        || format!("{:?}", session.state()),
        || async { matches!(session.state(), oxikube_app::logs::LogState::Failed(_)) },
    )
    .await;
}
