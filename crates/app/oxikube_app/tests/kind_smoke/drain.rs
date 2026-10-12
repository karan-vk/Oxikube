//! Node drain (E12-S10) end to end: the drain dialog's flow (`DrainFlow`) dispatches
//! `node::Drain` on a `CommandBus` with the real `MutationGuard`, over a session of the real
//! `KubeConnector`, and the app's `NodeService` drains through the kube adapter's `KubeDrain`.
//!
//! A PodDisruptionBudget (`minAvailable: 2`) first holds both pods back; the drain shows the pod
//! as blocked within seconds (the connector's evictions use a client without retries, so the
//! budget's 429 is not retried for minutes underneath), then the budget is deleted, as an
//! operator would, and the drain finishes. Read-only refuses a drain before anything is read.
//!
//! The shared kind node is never touched: the node is a fake Node object no kubelet backs
//! (unique per test, tainted, deleted at the end) and the pods live in the test's own namespace,
//! bound to it by `spec.nodeName`, made Running and Ready by writing their status (what a budget
//! counts), and evicted with `gracePeriodSeconds: 0` (no kubelet would confirm a graceful delete).

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use kube::Api;
use kube::api::{DeleteParams, Patch, PatchParams, PostParams};
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError};
use oxikube_app::nodes::{DrainFlow, NodeService, register_drain_command};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{CommandBus, MutationGuard};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::drain::{DrainFlags, DrainProgress};
use oxikube_kube::{ConnectorConfig, KubeConnector, KubeDrain, PoolConfig};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use serde_json::json;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};
use crate::nodes::{FakeNode, node_ref, unschedulable};

/// How long the first "blocked by the budget" may take to show.
const BLOCKED_WITHIN: Duration = Duration::from_secs(30);

/// Creates `name` on `node` labelled `app=web`, then writes a Running, Ready status for it.
async fn ready_pod(client: &kube::Client, namespace: &str, name: &str, node: &str) {
    let pods = Api::<Pod>::namespaced(client.clone(), namespace);
    let pod: Pod = serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "labels": {"app": "web"}},
        "spec": {
            "nodeName": node,
            "terminationGracePeriodSeconds": 0,
            "containers": [{"name": "pause", "image": oxikube_testkit::images::PAUSE}],
        },
    }))
    .expect("pod json");
    pods.create(&PostParams::default(), &pod)
        .await
        .expect("create a bound pod");
    pods.patch_status(
        name,
        &PatchParams::default(),
        &Patch::Merge(json!({"status": {
            "phase": "Running",
            "conditions": [{"type": "Ready", "status": "True"}],
        }})),
    )
    .await
    .expect("make the pod Ready");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_drain_through_the_guard_waits_out_a_disruption_budget_then_finishes() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let node = FakeNode::create(
        &client,
        kind.context.as_str(),
        format!("oxi-drain-{}", ns.name()),
    )
    .await;
    for name in ["web-1", "web-2"] {
        ready_pod(&client, ns.name(), name, &node.name).await;
    }
    let budgets = Api::<PodDisruptionBudget>::namespaced(client.clone(), ns.name());
    let budget: PodDisruptionBudget = serde_json::from_value(json!({
        "apiVersion": "policy/v1", "kind": "PodDisruptionBudget",
        "metadata": {"name": "keep-web"},
        "spec": {"minAvailable": 2, "selector": {"matchLabels": {"app": "web"}}},
    }))
    .expect("pdb json");
    budgets
        .create(&PostParams::default(), &budget)
        .await
        .expect("create the budget");
    crate::eventually("the budget to count both pods", String::new, || async {
        budgets
            .get("keep-web")
            .await
            .ok()
            .and_then(|pdb| pdb.status)
            .is_some_and(|s| s.current_healthy == Some(2) && s.disruptions_allowed == Some(0))
    })
    .await;

    // The app as the binary wires it: the connector's sessions, the guard, the node service over
    // the adapter's drain, the dialog's flow.
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
    let nodes = Arc::new(NodeService::new(
        Arc::new(KubeDrain),
        manager.clone(),
        Arc::new(TokioClock),
    ));
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::nodes", |r| {
            register_drain_command(r, nodes.clone())
        })
        .expect("register node::Drain");
    let state = Arc::new(FakeStatePort::new());
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
    );
    nodes.set_audit(bus.guard().audit_handle());
    let flow = DrainFlow::new(bus.clone(), nodes.clone(), "tester");
    let target = node_ref(&entry.cluster, &node.name);

    // Read-only refuses the drain before anything is read or written.
    manager.set_read_only(&entry.cluster, true).unwrap();
    let refused = bus
        .dispatch(
            Command::NodeDrain {
                target: target.clone(),
                flags: DrainFlags::default(),
            },
            DispatchContext::new(Initiator::Agent, "tester"),
        )
        .await
        .expect_err("read-only refuses");
    assert!(
        matches!(refused, DispatchError::ReadOnly { .. }),
        "{refused:?}"
    );
    assert_eq!(unschedulable(&client, &node.name).await, None);
    manager.set_read_only(&entry.cluster, false).unwrap();

    // The plan the dialog shows first: both pods would be evicted once forced (no controller).
    let context = flow.open(&target).await.expect("read the node's pods");
    assert_eq!(context.pods.len(), 2);
    let flags = DrainFlags {
        force: true,
        grace_period_secs: Some(0),
        timeout_secs: 120,
        ..DrainFlags::default()
    };
    assert_eq!(context.plan(&DrainFlags::default()).blocked.len(), 2);
    assert_eq!(context.plan(&flags).evict.len(), 2);

    let start = flow.start(&target, flags, node.name.clone());
    let run = tokio::spawn(start.run);
    let mut progress = start.progress;
    let started = Instant::now();
    let mut seen = Vec::new();
    let mut relaxed = false;
    while let Some(step) = progress.next().await {
        if matches!(step, DrainProgress::Blocked { .. }) && !relaxed {
            assert!(
                started.elapsed() < BLOCKED_WITHIN,
                "the budget's refusal took {:?} to show",
                started.elapsed()
            );
            relaxed = true;
            budgets
                .delete("keep-web", &DeleteParams::default())
                .await
                .expect("delete the budget");
        }
        seen.push(step);
    }
    let message = run.await.expect("joined").expect("the drain finishes");
    assert!(message.starts_with("Drained node"), "{message}");
    assert!(relaxed, "the budget never blocked: {seen:?}");
    assert!(matches!(seen[0], DrainProgress::Planned { .. }));
    let Some(DrainProgress::Finished(summary)) = seen.last() else {
        panic!("not finished: {seen:?}");
    };
    assert!(summary.is_complete(), "{summary:?}");
    assert_eq!(summary.evicted.len(), 2);
    assert_eq!(unschedulable(&client, &node.name).await, Some(true));
    let left = Api::<Pod>::namespaced(client.clone(), ns.name())
        .list(&Default::default())
        .await
        .expect("list pods");
    assert!(left.items.is_empty(), "every pod went");

    // The audit: the denial, then one record per pod and the drain's own, sharing the batch.
    let audit = state.audit_log();
    assert_eq!(audit[0].outcome, AuditOutcome::Denied);
    let drain = audit.last().expect("the drain's record");
    assert_eq!(drain.outcome, AuditOutcome::Succeeded);
    assert_eq!(drain.target, target);
    let detail = drain.detail.as_deref().unwrap_or_default();
    let batch = detail
        .split(' ')
        .find(|w| w.starts_with("batch="))
        .expect("a batch id")
        .to_owned();
    assert!(
        detail.contains("force=true") && detail.contains("evicted=2"),
        "{detail}"
    );
    let pods: Vec<_> = audit
        .iter()
        .filter(|r| &*r.target.gvk.kind == "Pod")
        .collect();
    assert_eq!(pods.len(), 2);
    assert!(pods.iter().all(|r| {
        r.outcome == AuditOutcome::Succeeded
            && r.detail.as_deref().is_some_and(|d| d.contains(&batch))
    }));
    drop(node);
    drop(ns);
}
