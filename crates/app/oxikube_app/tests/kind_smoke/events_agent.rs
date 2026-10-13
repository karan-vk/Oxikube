//! The agent's events hooks (E14-S08) over the real adapters: `k8s.get_events` through the
//! `ToolRegistry`, `@events` through the `ContextRegistry` and `events::SendToAgent` on the
//! command bus, reading a pod no node can take (`FailedScheduling`, a `Warning`) and a pod that
//! starts, through the `EventService` whose one cluster-wide feed the first call starts on
//! demand.
//!
//! * the tool lists the stuck pod's warning by kind and name, by uid, and by `ns`/`type`, and
//!   leaves the running pod's events out of a warnings query;
//! * `@events/pod/<ns>/<name>` reads the pod (for its uid), finds its events through the index
//!   and says so for a pod that does not exist;
//! * `events::SendToAgent` queues the pod's events with their source.
//!
//! Everything lives in the test's own `oxi-test-<rand>` namespace; the feed is cluster-wide (as
//! in the app), so every query is limited to it.

use std::sync::Arc;

use futures::future::BoxFuture;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::PostParams;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext, Outcome};
use oxikube_app::context::{CollectingConsumer, ContextRegistry, PendingContext};
use oxikube_app::events::{EventService, EventsConfig};
use oxikube_app::guard::MutationGuard;
use oxikube_app::search::aliases::AliasRegistry;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::store::Spawner;
use oxikube_app::tools::ToolRegistry;
use oxikube_app::tools::events::{EventsAgentDeps, install, register_commands};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_ports::{ContextScope, ToolContext};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use serde_json::{Value, json};

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
    let mut pod: Value = oxikube_testkit::integration::pods::sleeper(name);
    pod["spec"]["containers"][0]["resources"] = json!({ "requests": { "cpu": "1000" } });
    serde_json::from_value(pod).expect("pod json")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_tool_the_mention_and_send_to_agent_read_the_real_clusters_events() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    let running: Pod =
        serde_json::from_value(oxikube_testkit::integration::pods::sleeper("ag-run"))
            .expect("pod json");
    pods.create(&PostParams::default(), &running)
        .await
        .expect("create the sleeper");
    pods.create(&PostParams::default(), &unschedulable("ag-stuck"))
        .await
        .expect("create the unschedulable pod");

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

    // Nobody subscribes first: the tool's first call starts the cluster's feed.
    let events = EventService::new(
        sessions.clone(),
        Arc::new(TokioClock),
        EventsConfig::default(),
    );
    events.start(tokio_spawner());
    assert!(!events.is_watching(&cluster));
    let deps = EventsAgentDeps {
        sessions: sessions.clone(),
        events: events.clone(),
        aliases: AliasRegistry::new(),
        clock: Arc::new(TokioClock),
    };
    let (tools, contexts) = (Arc::new(ToolRegistry::new()), ContextRegistry::new());
    install(&tools, &contexts, &deps).expect("install");
    let tool_ctx = ToolContext::agent().with_cluster(cluster.clone());
    let query = |args: Value| {
        let (tools, ctx) = (tools.clone(), tool_ctx.clone());
        async move {
            tools
                .invoke("k8s.get_events", args, &ctx)
                .await
                .expect("a valid call")
        }
    };
    let diagnostics = || format!("feed {:?}", events.state(&cluster));

    // By kind and name: the scheduler's warning on the stuck pod.
    let by_name = || {
        query(json!({
            "ns": ns.name(),
            "involved": {"kind": "pod", "name": "ag-stuck"},
        }))
    };
    eventually("FailedScheduling through the tool", diagnostics, || async {
        let out = by_name().await;
        out.structured.is_some_and(|s| {
            s["events"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|r| r["reason"] == "FailedScheduling"))
        })
    })
    .await;
    let last = by_name().await.structured.expect("structured");
    assert!(events.is_watching(&cluster), "the call started the feed");
    assert_eq!(last["status"], "ok");
    let warning = last["events"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["reason"] == "FailedScheduling"))
        .expect("the warning");
    assert_eq!(warning["type"], "Warning");
    assert_eq!(warning["object"], format!("Pod/{}/ag-stuck", ns.name()));

    // A warnings query leaves the running pod's Normal events out.
    let warnings = query(json!({"ns": ns.name(), "type": "Warning"})).await;
    let rows = warnings.structured.expect("structured")["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|r| r["type"] == "Warning"), "{rows:?}");
    assert!(
        rows.iter()
            .all(|r| r["object"] != format!("Pod/{}/ag-run", ns.name())),
        "{rows:?}"
    );

    // The mention reads the pod for its uid and finds the events through the index.
    let scope = ContextScope::new().with_cluster(cluster.clone());
    let blocks = contexts
        .resolve_text(&format!("@events/pod/{}/ag-stuck", ns.name()), &scope)
        .await
        .expect("a valid mention");
    assert!(
        blocks[0].body.contains("FailedScheduling"),
        "{}",
        blocks[0].body
    );
    let missing = contexts
        .resolve_text(&format!("@events/pod/{}/nope", ns.name()), &scope)
        .await
        .expect("a valid mention");
    assert!(
        missing[0].body.contains("# error: Pod/") && missing[0].body.contains("was not found"),
        "{}",
        missing[0].body
    );

    // Send to agent, on a bus.
    let pending = PendingContext::new();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::tools::events", |r| {
            register_commands(r, &deps, pending.clone())
        })
        .expect("register");
    let guard = MutationGuard::new(
        sessions.clone(),
        Arc::new(FakeStatePort::new()),
        Arc::new(TokioClock),
    );
    let bus = CommandBus::new(registry, guard);
    let stuck = ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("", "v1", "Pod"),
        ns.name(),
        "ag-stuck",
    );
    let outcome = bus
        .dispatch(
            Command::EventsSendToAgent {
                cluster: cluster.clone(),
                target: Some(stuck),
                events: Vec::new(),
            },
            DispatchContext::new(Initiator::Ui, "me"),
        )
        .await
        .expect("dispatched");
    assert!(matches!(outcome, Outcome::Completed(_)), "{outcome:?}");
    let consumer = CollectingConsumer::new();
    let _attached = pending.attach(consumer.clone());
    let items = consumer.take();
    assert_eq!(items.len(), 1);
    assert!(items[0].block.body.contains("FailedScheduling"));
    assert_eq!(items[0].source.namespace, ns.name());
}
