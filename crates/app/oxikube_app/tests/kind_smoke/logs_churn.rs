//! Reconnect and churn following (E08-S07) over the real adapters: a Deployment of three busybox
//! pods that echo their own name and a counter, read as one merged log through
//! `LogService::open_aggregate` and, for one of its pods, as a single-pod session through
//! `LogService::open_following_in`; then `kubectl rollout restart` (the same patch: a new
//! `restartedAt` annotation on the pod template).
//!
//! * the merged view picks up the new pods and reads them from their first line, even though it
//!   was opened on a 5-line tail;
//! * the old pods end (`PodChange::Ended`) and their lines stay;
//! * no line is in the buffer twice;
//! * the single-pod session ends `PodReplaced`, and `find_replacement` names a pod of the new
//!   ReplicaSet.
//!
//! Everything lives in the test's own `oxi-test-<rand>` namespace.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::{ListParams, Patch, PatchParams, PostParams};
use oxikube_app::logs::{
    AggregatePorts, AggregateSpec, EndReason, LogConfig, LogRuntime, LogService, LogState,
    LogTarget, PodChange, find_replacement,
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

/// A rollout on a shared single-node cluster: image already present, grace period 0, but the
/// scheduler and kubelet serve every suite at once.
const ROLLOUT_DEADLINE: Duration = Duration::from_secs(120);

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

/// The names of the running pods of the deployment that are not being deleted.
async fn running_pods(pods: &Api<Pod>) -> BTreeSet<String> {
    pods.list(&ListParams::default().labels("app=web"))
        .await
        .map(|list| {
            list.items
                .into_iter()
                .filter(|p| p.metadata.deletion_timestamp.is_none())
                .filter(|p| {
                    p.status
                        .as_ref()
                        .and_then(|s| s.phase.as_deref())
                        .is_some_and(|phase| phase == "Running")
                })
                .filter_map(|p| p.metadata.name)
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rollout_restart_keeps_the_workload_view_on_the_new_pods_without_duplicates() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let deployments = Api::<Deployment>::namespaced(client.clone(), ns.name());
    deployments
        .create(&PostParams::default(), &deployment())
        .await
        .expect("create the deployment");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    eventually("three running pods", String::new, || async {
        running_pods(&pods).await.len() == 3
    })
    .await;
    let old = running_pods(&pods).await;

    let resources = Arc::new(KubeResources::new(
        client.clone(),
        KubeDiscovery::new(client.clone()),
    ));
    let ports = AggregatePorts {
        logs: Arc::new(KubeLogs::new(client.clone())),
        resources: resources.clone(),
    };
    let cluster: ClusterId = catalog_entry(&kind.context).cluster;
    let target = ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("apps", "v1", "Deployment"),
        ns.name(),
        "web",
    );
    let service = service();
    // A short tail: the pods of the first list start from their last lines, the new ones must not.
    let merged = service.open_aggregate(
        ports.clone(),
        AggregateSpec::of(&target).expect("a deployment"),
        LogOptions::follow().tail_lines(5),
    );
    let view = merged.aggregate().clone();
    let followed = old.iter().next().expect("a pod").clone();
    let single = service.open_following_in(
        &cluster,
        ports,
        LogTarget::pod(ns.name(), &followed),
        LogOptions::follow().tail_lines(5),
    );
    eventually(
        "lines of the three pods and of the followed one",
        || format!("{:?} / {:?}", merged.state(), single.state()),
        || async {
            let seen: HashSet<String> =
                merged.read(|b, _| b.iter().map(|e| e.pod.to_string()).collect());
            seen.len() == 3 && !single.is_empty()
        },
    )
    .await;

    // `kubectl rollout restart deployment/web`.
    let restart = serde_json::json!({"spec": {"template": {"metadata": {"annotations": {
        "kubectl.kubernetes.io/restartedAt": jiff::Timestamp::now().to_string()
    }}}}});
    deployments
        .patch("web", &PatchParams::default(), &Patch::Merge(&restart))
        .await
        .expect("restart the rollout");

    let started = Instant::now();
    loop {
        let new_pods: BTreeSet<String> = merged.read(|b, _| {
            b.iter()
                .map(|e| e.pod.to_string())
                .filter(|pod| !old.contains(pod))
                .collect()
        });
        let ended: HashSet<String> = view
            .events_after(None)
            .iter()
            .filter(|e| e.change == PodChange::Ended)
            .map(|e| e.pod.to_string())
            .collect();
        let done = new_pods.len() >= 3
            && old.iter().all(|pod| ended.contains(pod))
            && single.state().is_terminal();
        if done {
            break;
        }
        assert!(
            started.elapsed() < ROLLOUT_DEADLINE,
            "the rollout was not followed: new pods read {new_pods:?}, old pods ended {ended:?}, \
             merged {:?}, single {:?}, sources {:?}",
            merged.state(),
            single.state(),
            view.sources()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    // Let the new pods' backlog land.
    tokio::time::sleep(Duration::from_secs(2)).await;

    assert_eq!(
        merged.state(),
        LogState::Streaming,
        "the view keeps following"
    );
    merged.read(|buffer, _| {
        let mut seen = HashSet::new();
        for entry in buffer.iter() {
            assert!(
                seen.insert((entry.pod.clone(), entry.text.clone())),
                "{}: {:?} is in the buffer twice",
                entry.pod,
                entry.text
            );
            let host = entry.text.split(' ').next().unwrap_or_default();
            assert_eq!(host, &*entry.pod, "every line is its own pod's");
        }
        let pods: BTreeSet<String> = buffer.iter().map(|e| e.pod.to_string()).collect();
        for pod in &old {
            assert!(pods.contains(pod), "the old pod {pod}'s lines stay");
        }
        for pod in pods.iter().filter(|pod| !old.contains(*pod)) {
            let first = buffer
                .iter()
                .find(|e| &*e.pod == pod.as_str())
                .map(|e| e.text.to_string());
            assert_eq!(
                first.as_deref(),
                Some(format!("{pod} 0").as_str()),
                "a new pod is read from its first line"
            );
        }
    });

    // The single-pod session says its pod was replaced, and the replacement is a new pod.
    assert_eq!(single.state(), LogState::Ended(EndReason::PodReplaced));
    let gone = single.pod_identity().expect("the followed pod was read");
    assert_eq!(gone.name, followed);
    let replacement = find_replacement(resources.as_ref(), &gone)
        .await
        .expect("the lookup reads")
        .expect("a replacement");
    assert!(
        !old.contains(&replacement),
        "{replacement} is a pod of the new ReplicaSet"
    );
}
