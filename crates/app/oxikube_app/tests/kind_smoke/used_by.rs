//! "Used by" (E12-S18) against the real adapter: the reference index over a kind cluster's Pods
//! and the owner walk through the API server, no UI.
//!
//! In the test's own `oxi-test-<rand>` namespace a Deployment, a Job and a bare Pod all use the
//! ConfigMap `cfg`; the Deployment also uses the Secret `tls` and the ServiceAccount `runner`.
//! The feed reads the Pods through the `ResourceStore`; `used_by` walks Pod to ReplicaSet to
//! Deployment with real `ownerReferences`, lists the Job (no owner of its own) and the Pod that
//! lists itself, follows a deleted Pod, and releases the Pods subscription with the feed.
//! Pods are counted while Pending, so nothing waits for an image; nothing touches the node.

use std::sync::Arc;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{ConfigMap, Pod, Secret, ServiceAccount};
use kube::Api;
use kube::api::{DeleteParams, PostParams};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::store::{ResourceStore, Spawner, StoreOptions, StorePorts, StoreRuntime};
use oxikube_app::{ReferenceFeed, UsedBy, WorkloadUse, used_by};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::view::RefKind;
use oxikube_kube::{ConnectorConfig, KubeConnector, KubeDiscovery, KubeResources, PoolConfig};
use oxikube_testkit::FakeClusterSourcePort;
use oxikube_testkit::images::BUSYBOX;
use oxikube_testkit::integration::TestNamespace;
use serde_json::{Value, json};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

fn container() -> Value {
    json!({
        "name": "main", "image": BUSYBOX, "command": ["sleep", "3600"],
        "envFrom": [{"configMapRef": {"name": "cfg"}}],
    })
}

fn spec(extra: Value) -> Value {
    let mut spec = json!({"terminationGracePeriodSeconds": 0, "containers": [container()]});
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        spec[key] = value;
    }
    spec
}

fn deployment() -> Deployment {
    serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "web"},
        "spec": {
            "replicas": 2,
            "selector": {"matchLabels": {"app": "web"}},
            "template": {
                "metadata": {"labels": {"app": "web"}},
                "spec": spec(json!({
                    "serviceAccountName": "runner",
                    "volumes": [{"name": "tls", "secret": {"secretName": "tls"}}],
                })),
            },
        },
    }))
    .expect("a deployment")
}

fn job() -> Job {
    serde_json::from_value(json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": "batchy"},
        "spec": {"template": {"spec": spec(json!({"restartPolicy": "Never"}))}},
    }))
    .expect("a job")
}

fn lone_pod() -> Pod {
    serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "lone"},
        "spec": spec(json!({})),
    }))
    .expect("a pod")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_index_and_the_owner_walk_against_a_kind_cluster() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let create = PostParams::default();
    Api::<ConfigMap>::namespaced(admin.clone(), ns.name())
        .create(
            &create,
            &serde_json::from_value(json!({"metadata": {"name": "cfg"}, "data": {"k": "v"}}))
                .expect("a config map"),
        )
        .await
        .expect("create the config map");
    Api::<Secret>::namespaced(admin.clone(), ns.name())
        .create(
            &create,
            &serde_json::from_value(
                json!({"metadata": {"name": "tls"}, "stringData": {"k": "not-a-real-secret"}}),
            )
            .expect("a secret"),
        )
        .await
        .expect("create the secret");
    Api::<ServiceAccount>::namespaced(admin.clone(), ns.name())
        .create(
            &create,
            &serde_json::from_value(json!({"metadata": {"name": "runner"}}))
                .expect("a service account"),
        )
        .await
        .expect("create the service account");
    Api::<Deployment>::namespaced(admin.clone(), ns.name())
        .create(&create, &deployment())
        .await
        .expect("create the deployment");
    Api::<Job>::namespaced(admin.clone(), ns.name())
        .create(&create, &job())
        .await
        .expect("create the job");
    let pods = Api::<Pod>::namespaced(admin.clone(), ns.name());
    pods.create(&create, &lone_pod())
        .await
        .expect("create the pod");

    let entry = catalog_entry(&kind.context);
    let connector = KubeConnector::new(
        kind.kubeconfig.clone(),
        PoolConfig::default(),
        ConnectorConfig::default(),
    );
    let manager = ClusterSessionManager::with_config(
        Arc::new(connector),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
        SessionManagerConfig::default(),
    );
    manager.open(&entry, SessionOptions::default());
    manager.connect(&entry.cluster).await.expect("connect");
    manager
        .set_namespace_selection(&entry.cluster, NamespaceSelection::single(ns.name()))
        .unwrap();
    let session = manager.get(&entry.cluster).expect("the session");
    let reader = Arc::new(KubeResources::new(
        admin.clone(),
        KubeDiscovery::new(admin.clone()),
    ));
    let spawner: Arc<dyn Spawner> = Arc::new(|task| {
        tokio::spawn(task);
    });
    let store = ResourceStore::new(
        session.id().clone(),
        StorePorts {
            resources: reader.clone(),
            tables: reader.clone(),
        },
        StoreRuntime {
            spawner,
            clock: Arc::new(TokioClock),
            probe: None,
        },
        StoreOptions::default(),
    );
    let feed = ReferenceFeed::start(&store, &manager);
    let (index, owners) = (feed.index().clone(), feed.owners().clone());
    let cluster = entry.cluster.clone();
    let target =
        |gvk: Gvk, name: &str| ResourceRef::namespaced(cluster.clone(), gvk, ns.name(), name);
    let ask = |kind: RefKind, name: &'static str| {
        let (index, owners, reader, cluster) = (
            index.clone(),
            owners.clone(),
            reader.clone(),
            cluster.clone(),
        );
        let namespace = ns.name().to_owned();
        async move {
            used_by(
                &index,
                &owners,
                &*reader,
                &cluster,
                kind,
                Some(&namespace),
                name,
            )
            .await
        }
    };
    let web = target(Gvk::new("apps", "v1", "Deployment"), "web");
    let batchy = target(Gvk::new("batch", "v1", "Job"), "batchy");
    let lone = target(Gvk::new("", "v1", "Pod"), "lone");

    // The ConfigMap is used by the Deployment (two pods, found through the ReplicaSet), the Job and
    // the lone Pod.
    crate::eventually(
        "the ConfigMap's users to be found",
        || format!("{index:?}"),
        || async {
            matches!(
                ask(RefKind::ConfigMap, "cfg").await,
                UsedBy::Known { pods: 4, .. }
            )
        },
    )
    .await;
    let UsedBy::Known {
        workloads,
        pods: counted,
        ..
    } = ask(RefKind::ConfigMap, "cfg").await
    else {
        panic!("known");
    };
    assert_eq!(counted, 4);
    assert_eq!(
        workloads,
        [
            WorkloadUse {
                target: web.clone(),
                pods: 2
            },
            WorkloadUse {
                target: batchy,
                pods: 1
            },
            WorkloadUse {
                target: lone.clone(),
                pods: 1
            },
        ]
    );
    assert!(!owners.is_empty(), "the owner chains are remembered");

    // The Secret and the ServiceAccount are the Deployment's alone.
    for (kind, name) in [
        (RefKind::Secret, "tls"),
        (RefKind::ServiceAccount, "runner"),
    ] {
        let UsedBy::Known { workloads, .. } = ask(kind, name).await else {
            panic!("known");
        };
        assert_eq!(
            workloads,
            [WorkloadUse {
                target: web.clone(),
                pods: 2
            }],
            "{name}"
        );
    }

    // A deleted Pod stops using what it named.
    pods.delete("lone", &DeleteParams::default().grace_period(0))
        .await
        .expect("delete the pod");
    crate::eventually(
        "the deleted pod to be forgotten",
        || format!("{index:?}"),
        || async {
            matches!(
                ask(RefKind::ConfigMap, "cfg").await,
                UsedBy::Known { pods: 3, .. }
            )
        },
    )
    .await;
    drop(feed);
}
