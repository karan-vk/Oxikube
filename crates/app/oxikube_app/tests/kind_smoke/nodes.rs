//! Nodes (E12-S09) against the real adapter: `node::Cordon` and `node::Uncordon` through the
//! `CommandBus` and `MutationGuard`, the pods on a node listed by field selector, and the
//! store's pods-by-node index over the real Pod feed.
//!
//! The single kind node is shared with every other suite, so nothing here cordons, taints or
//! drains it: the node under test is a fake Node object no kubelet backs (created tainted
//! `NoSchedule`, named after the test's own namespace, deleted when the test ends), and the pods
//! live in the test's own `oxi-test-<rand>` namespace, bound to the fake node by `spec.nodeName`
//! (no scheduler involved).

use std::process::Command as Process;
use std::sync::Arc;

use k8s_openapi::api::core::v1::{Node, Pod};
use kube::Api;
use kube::api::PostParams;
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::nodes::{list_node_pods, register_commands};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::store::{ResourceStore, Spawner, StoreOptions, StorePorts, StoreRuntime};
use oxikube_app::{CommandBus, MutationGuard};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, KubeDiscovery, KubeResources, PoolConfig};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClockPort, FakeClusterSourcePort, FakeStatePort};
use serde_json::json;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// A Node no kubelet backs, deleted (best effort) when dropped.
struct FakeNode {
    context: String,
    name: String,
}

impl FakeNode {
    async fn create(client: &kube::Client, context: &str, name: String) -> Self {
        let node: Node = serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "Node",
            "metadata": {"name": name},
            "spec": {"taints": [{"key": "oxikube.test/fake", "effect": "NoSchedule"}]},
        }))
        .expect("node json");
        let guard = Self {
            context: context.to_owned(),
            name,
        };
        Api::<Node>::all(client.clone())
            .create(&PostParams::default(), &node)
            .await
            .expect("create the fake node");
        guard
    }
}

impl Drop for FakeNode {
    fn drop(&mut self) {
        let _ = Process::new("kubectl")
            .args(["--context", &self.context, "delete", "node", &self.name])
            .args(["--ignore-not-found", "--wait=false"])
            .output();
    }
}

fn node_ref(cluster: &ClusterId, name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(cluster.clone(), Gvk::new("", "v1", "Node"), name)
}

async fn unschedulable(client: &kube::Client, name: &str) -> Option<bool> {
    Api::<Node>::all(client.clone())
        .get(name)
        .await
        .expect("get the node")
        .spec
        .and_then(|spec| spec.unschedulable)
}

/// A pod bound to `node` by name, requesting `cpu` and `memory`.
async fn bind_pod(client: &kube::Client, namespace: &str, name: &str, node: &str, cpu: &str) {
    let pod: Pod = serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name},
        "spec": {
            "nodeName": node,
            "terminationGracePeriodSeconds": 1,
            "containers": [{
                "name": "pause",
                "image": oxikube_testkit::images::PAUSE,
                "resources": {
                    "requests": {"cpu": cpu, "memory": "64Mi"},
                    "limits": {"cpu": cpu, "memory": "128Mi"}
                }
            }]
        }
    }))
    .expect("pod json");
    Api::<Pod>::namespaced(client.clone(), namespace)
        .create(&PostParams::default(), &pod)
        .await
        .expect("create a bound pod");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cordon_and_uncordon_a_fake_node_through_the_guard() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let node = FakeNode::create(
        &client,
        kind.context.as_str(),
        format!("oxi-fake-{}", ns.name()),
    )
    .await;

    let entry = catalog_entry(&kind.context);
    let manager = ClusterSessionManager::with_config(
        Arc::new(KubeConnector::new(
            kind.kubeconfig.clone(),
            PoolConfig::default(),
            ConnectorConfig::default(),
        )),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
        SessionManagerConfig::default(),
    );
    manager.open(&entry, SessionOptions::default());
    manager.connect(&entry.cluster).await.expect("connect");
    let state = Arc::new(FakeStatePort::new());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::nodes", register_commands)
        .expect("register the node commands");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
    );
    let target = node_ref(&entry.cluster, &node.name);
    let run = |command: Command, dry_run: bool| {
        let bus = bus.clone();
        async move {
            let ctx = DispatchContext::new(Initiator::Ui, "tester").with_dry_run(dry_run);
            let Ok(Outcome::NeedsConfirmation(request)) =
                bus.dispatch(command.clone(), ctx.clone()).await
            else {
                panic!("a confirmation is asked first");
            };
            assert_eq!(request.expected_name, None, "one click: risk low");
            bus.dispatch(
                command,
                ctx.with_confirmation(Confirmation::simple(request.token)),
            )
            .await
        }
    };
    let cordon = || Command::NodeCordon {
        target: target.clone(),
    };
    let uncordon = || Command::NodeUncordon {
        target: target.clone(),
    };

    assert_eq!(unschedulable(&client, &node.name).await, None);

    // A dry-run dispatch changes nothing.
    run(cordon(), true).await.expect("dry run");
    assert_eq!(unschedulable(&client, &node.name).await, None);

    // Cordon: `spec.unschedulable` is set on the node (the server's dry run passed first).
    run(cordon(), false).await.expect("cordon");
    assert_eq!(unschedulable(&client, &node.name).await, Some(true));

    // Read-only refuses both, before any request.
    manager.set_read_only(&entry.cluster, true).unwrap();
    let err = run_denied(&bus, uncordon()).await;
    assert_eq!(err.read_only_cluster(), Some(&entry.cluster));
    assert_eq!(unschedulable(&client, &node.name).await, Some(true));
    manager.set_read_only(&entry.cluster, false).unwrap();

    // Uncordon removes the field.
    run(uncordon(), false).await.expect("uncordon");
    assert_eq!(unschedulable(&client, &node.name).await, None);

    // The audit log: dry run, cordon, the denial, uncordon, each with the node and the initiator.
    let audit = state.audit_log();
    let seen: Vec<_> = audit
        .iter()
        .map(|r| (&*r.cmd, r.dry_run, r.outcome, r.initiator))
        .collect();
    assert_eq!(
        seen,
        [
            ("node::Cordon", true, AuditOutcome::Succeeded, Initiator::Ui),
            (
                "node::Cordon",
                false,
                AuditOutcome::Succeeded,
                Initiator::Ui
            ),
            ("node::Uncordon", false, AuditOutcome::Denied, Initiator::Ui),
            (
                "node::Uncordon",
                false,
                AuditOutcome::Succeeded,
                Initiator::Ui
            ),
        ]
    );
    assert!(audit.iter().all(|r| r.target == target));
    drop(ns);
}

/// Dispatches `command` on a read-only cluster; the guard refuses it before a confirmation.
async fn run_denied(bus: &CommandBus, command: Command) -> DispatchError {
    bus.dispatch(command, DispatchContext::new(Initiator::Ui, "tester"))
        .await
        .expect_err("read-only refuses")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_pods_on_a_node_by_field_selector_and_by_the_stores_index() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let node = FakeNode::create(
        &client,
        kind.context.as_str(),
        format!("oxi-fake-{}", ns.name()),
    )
    .await;
    bind_pod(&client, ns.name(), "a", &node.name, "250m").await;
    bind_pod(&client, ns.name(), "b", &node.name, "500m").await;

    let reader = Arc::new(KubeResources::new(
        client.clone(),
        KubeDiscovery::new(client.clone()),
    ));

    // The fallback: the server filters by `spec.nodeName` (and drops finished pods).
    let found = list_node_pods(&*reader, &node.name).await.expect("list");
    let names: Vec<_> = found.pods.iter().map(|p| p.meta.name.to_string()).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(found.demand.pods, 2);
    assert_eq!(found.demand.cpu_requests, 750);
    assert_eq!(found.demand.cpu_limits, 750);
    assert_eq!(found.demand.memory_requests, 128 << 20);
    assert_eq!(found.demand.memory_limits, 256 << 20);

    // The store's index over the real cluster-wide Pod feed gives the same answer.
    let spawner: Arc<dyn Spawner> = Arc::new(|task| {
        tokio::spawn(task);
    });
    let store = ResourceStore::new(
        ClusterId::new("kind-smoke", &kind.context),
        StorePorts {
            resources: reader.clone(),
            tables: reader,
        },
        StoreRuntime {
            spawner,
            clock: Arc::new(FakeClockPort::default()),
            probe: None,
        },
        StoreOptions::default(),
    );
    let lease = store.lease_node_loads();
    crate::eventually(
        "the pods-by-node index to list the pods",
        String::new,
        || async {
            lease
                .demand(&node.name)
                .is_some_and(|demand| demand.pods == 2)
        },
    )
    .await;
    let indexed = lease.demand(&node.name).expect("ready");
    assert_eq!(indexed, found.demand);
    let pods = lease.pods(&node.name).expect("ready");
    assert_eq!(pods.len(), 2);

    // A pod that goes leaves the index.
    Api::<Pod>::namespaced(client.clone(), ns.name())
        .delete("a", &kube::api::DeleteParams::default().grace_period(0))
        .await
        .expect("delete a pod");
    crate::eventually("the index to drop the deleted pod", String::new, || async {
        lease
            .demand(&node.name)
            .is_some_and(|demand| demand.pods == 1)
    })
    .await;
    assert_eq!(lease.demand(&node.name).unwrap().cpu_requests, 500);
    drop(ns);
}
