//! Node shells (E09-S09) through the guarded command and `ExecService` over the real adapter, on a
//! kind cluster: the pod is created on the node from the settings template, the shell runs in the
//! node's namespaces (`hostname` is the node's), closing the terminal deletes the pod and audits
//! it, a pod that cannot start is reported with advice and deleted, and a read-only cluster
//! creates nothing.
//!
//! The single kind node is shared with every other suite, so nothing here taints, cordons or
//! drains it: the tolerations of the template are checked on the pod the cluster admitted, and
//! the pods are short-lived and live in the test's own `oxi-test-<rand>` namespace, which has no
//! pod security labels.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::Api;
use kube::api::ListParams;
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError, Outcome};
use oxikube_app::exec::{NodeShellOpener, register_command};
use oxikube_app::guard::Confirmation;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{CommandBus, ExecService, MutationGuard};
use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_ports::{
    BackendEvent, ClusterPrefs, ClusterPrefsTable, NODE_SHELL_LABEL, NodeShellPrefs,
    NodeShellToleration,
};
use oxikube_testkit::images::BUSYBOX;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use parking_lot::Mutex;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

struct World {
    manager: ClusterSessionManager,
    bus: CommandBus,
    service: Arc<ExecService>,
    state: Arc<FakeStatePort>,
    opened: Arc<Mutex<Vec<ResourceRef>>>,
}

fn world(kind: &Kind) -> World {
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
    let service = Arc::new(ExecService::new(manager.clone()));
    let opened = Arc::new(Mutex::new(Vec::new()));
    let queue = opened.clone();
    let open: NodeShellOpener = Arc::new(move |node| {
        queue.lock().push(node.clone());
        Ok(())
    });
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::exec", |r| {
            register_command(r, service.clone(), open)
        })
        .expect("register node::Shell");
    let state = Arc::new(FakeStatePort::new());
    let guard = MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock));
    service.set_audit(guard.audit_handle());
    World {
        manager,
        bus: CommandBus::new(registry, guard),
        service,
        state,
        opened,
    }
}

/// The settings a user would have written: the shell pod in the test namespace from the cached
/// image, with a label and a toleration of its own.
fn prefs(namespace: &str, image: &str) -> ClusterPrefs {
    ClusterPrefs {
        node_shell_image: Some(image.to_owned()),
        node_shell: NodeShellPrefs {
            namespace: Some(namespace.to_owned()),
            labels: BTreeMap::from([("oxikube.test/suite".to_owned(), "node-shell".to_owned())]),
            tolerations: Some(vec![
                NodeShellToleration::everything(),
                NodeShellToleration {
                    key: Some("oxikube.test/never-set".into()),
                    operator: Some("Exists".into()),
                    effect: Some("NoSchedule".into()),
                    ..NodeShellToleration::default()
                },
            ]),
            max_lifetime_seconds: Some(600),
            ..NodeShellPrefs::default()
        },
        ..ClusterPrefs::default()
    }
}

async fn a_node(client: &kube::Client) -> String {
    Api::<Node>::all(client.clone())
        .list(&ListParams::default())
        .await
        .expect("list nodes")
        .items[0]
        .metadata
        .name
        .clone()
        .expect("node name")
}

async fn pods(client: &kube::Client, namespace: &str) -> Vec<Pod> {
    Api::<Pod>::namespaced(client.clone(), namespace)
        .list(&ListParams::default().labels(NODE_SHELL_LABEL))
        .await
        .expect("list pods")
        .items
        .into_iter()
        .filter(|pod| pod.metadata.deletion_timestamp.is_none())
        .collect()
}

async fn read_until(
    events: &mut futures::stream::BoxStream<'static, BackendEvent>,
    marker: &str,
) -> String {
    let mut text = String::new();
    let read = async {
        while let Some(event) = events.next().await {
            match event {
                BackendEvent::Output(bytes) => {
                    text.push_str(&String::from_utf8_lossy(&bytes));
                    if text.contains(marker) {
                        return;
                    }
                }
                BackendEvent::Exited(status) => panic!("the shell ended early: {status:?}\n{text}"),
                BackendEvent::Error(error) => panic!("the stream failed: {error}\n{text}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(120), read)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {marker:?}; read:\n{text}"));
    text
}

fn command(node: &ResourceRef) -> Command {
    Command::NodeShell {
        target: node.clone(),
    }
}

fn ui() -> DispatchContext {
    DispatchContext::new(Initiator::Ui, "kind-test")
}

/// Dispatches, confirms what the guard asks and runs.
async fn confirmed(bus: &CommandBus, node: &ResourceRef) -> Result<Outcome, DispatchError> {
    let Outcome::NeedsConfirmation(request) = bus.dispatch(command(node), ui()).await? else {
        panic!("a node shell is always confirmed");
    };
    bus.dispatch(
        command(node),
        ui().with_confirmation(Confirmation::simple(request.token)),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_node_shell_runs_on_the_node_and_closing_it_deletes_the_pod() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let node_name = a_node(&client).await;
    let w = world(&kind);
    let entry = catalog_entry(&kind.context);
    w.manager.open(&entry, SessionOptions::default());
    w.manager.connect(&entry.cluster).await.expect("connect");
    w.manager.set_prefs_table(
        ClusterPrefsTable::new(ClusterPrefs::default())
            .with_cluster(entry.cluster.clone(), prefs(ns.name(), BUSYBOX)),
    );
    let node = ResourceRef::cluster_scoped(
        entry.cluster.clone(),
        Gvk::new("", "v1", "Node"),
        node_name.as_str(),
    );

    // Asking confirms with the node and the image named; nothing exists yet.
    let Outcome::NeedsConfirmation(request) =
        w.bus.dispatch(command(&node), ui()).await.expect("asks")
    else {
        panic!("expected a confirmation");
    };
    assert!(request.summary.contains(&node_name), "{}", request.summary);
    assert!(request.summary.contains(BUSYBOX), "{}", request.summary);
    assert!(pods(&client, ns.name()).await.is_empty());
    w.bus
        .decline(request.token)
        .await
        .expect("declining is audited");

    // Confirmed: the cluster accepted the pod as a dry run, the terminal was asked for, and still
    // no pod exists: the session creates it.
    confirmed(&w.bus, &node).await.expect("runs");
    assert_eq!(w.opened.lock().as_slice(), std::slice::from_ref(&node));
    assert!(pods(&client, ns.name()).await.is_empty());

    let backend = w.service.open_node_shell(&node).await.expect("opens");
    let mut events = backend.output_stream();
    let notice = read_until(&mut events, "closes").await;
    assert!(notice.contains(&node_name), "{notice:?}");

    // The pod the cluster admitted is the template: privileged, on the node, labelled, with the
    // tolerations of the settings and a deadline.
    let created = pods(&client, ns.name()).await;
    assert_eq!(created.len(), 1, "one helper pod");
    let pod_name = created[0].metadata.name.clone().expect("name");
    let spec = created[0].spec.clone().expect("spec");
    assert_eq!(spec.node_name.as_deref(), Some(node_name.as_str()));
    assert_eq!(spec.active_deadline_seconds, Some(600));
    assert!(spec.host_pid == Some(true) && spec.host_network == Some(true));
    let tolerations = spec.tolerations.unwrap_or_default();
    assert!(
        tolerations
            .iter()
            .any(|t| t.key.as_deref() == Some("oxikube.test/never-set")),
        "the settings' tolerations are on the pod: {tolerations:?}"
    );
    assert_eq!(
        spec.containers[0]
            .security_context
            .as_ref()
            .and_then(|c| c.privileged),
        Some(true)
    );
    let labels = created[0].metadata.labels.clone().unwrap_or_default();
    assert_eq!(
        labels.get("oxikube.test/suite").map(String::as_str),
        Some("node-shell")
    );

    // Inside the node's namespaces the hostname is the node's, not the pod's.
    backend
        .write(b"echo host=$(hostname)=end\n")
        .await
        .expect("type");
    read_until(&mut events, &format!("host={node_name}=end")).await;

    // Closing the tab drops the backend: the pod is deleted and the end is audited.
    drop(events);
    drop(backend);
    crate::eventually(
        "the shell pod to be deleted",
        || format!("pods: {pod_name}"),
        || async { pods(&client, ns.name()).await.is_empty() },
    )
    .await;
    w.bus.guard().audit().flush().await.expect("flush");
    let audit = w.state.audit_log();
    let create = audit
        .iter()
        .find(|r| {
            r.detail
                .as_deref()
                .is_some_and(|d| d.starts_with("phase=create"))
        })
        .expect("the create record");
    let delete = audit
        .iter()
        .find(|r| {
            r.detail
                .as_deref()
                .is_some_and(|d| d.starts_with("phase=delete"))
        })
        .expect("the delete record");
    for record in [create, delete] {
        assert_eq!(&*record.cmd, "node::Shell");
        assert_eq!(&*record.target.name, node_name.as_str());
        assert_eq!(&*record.who, "kind-test");
        assert_eq!(record.initiator, Initiator::Ui);
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
        let detail = record.detail.as_deref().unwrap();
        assert!(
            detail.contains(BUSYBOX) && detail.contains(ns.name()),
            "{detail}"
        );
    }
    assert!(
        audit.iter().any(|r| r.outcome == AuditOutcome::Cancelled),
        "the declined confirmation"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_image_that_cannot_be_pulled_is_reported_with_advice_and_leaves_no_pod() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let node_name = a_node(&client).await;
    let w = world(&kind);
    let entry = catalog_entry(&kind.context);
    w.manager.open(&entry, SessionOptions::default());
    w.manager.connect(&entry.cluster).await.expect("connect");
    let bad = "registry.invalid/oxikube/none:0";
    w.manager.set_prefs_table(
        ClusterPrefsTable::new(ClusterPrefs::default())
            .with_cluster(entry.cluster.clone(), prefs(ns.name(), bad)),
    );
    let node = ResourceRef::cluster_scoped(
        entry.cluster,
        Gvk::new("", "v1", "Node"),
        node_name.as_str(),
    );

    confirmed(&w.bus, &node)
        .await
        .expect("the dry run accepts it");
    let Err(error) = w.service.open_node_shell(&node).await else {
        panic!("the image cannot be pulled");
    };
    assert_eq!(error.kind(), ErrorKind::Conflict, "{error}");
    assert!(error.message().contains(bad), "{error}");
    assert!(error.message().contains("node_shell_image"), "{error}");
    crate::eventually(
        "no shell pod is left",
        || String::new(),
        || async { pods(&client, ns.name()).await.is_empty() },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_only_cluster_and_a_missing_permit_create_nothing() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let node_name = a_node(&client).await;
    let w = world(&kind);
    let entry = catalog_entry(&kind.context);
    w.manager.open(&entry, SessionOptions::default());
    w.manager.connect(&entry.cluster).await.expect("connect");
    w.manager.set_prefs_table(
        ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            entry.cluster.clone(),
            ClusterPrefs {
                read_only: true,
                exec_in_read_only: true,
                ..prefs(ns.name(), BUSYBOX)
            },
        ),
    );
    let node = ResourceRef::cluster_scoped(
        entry.cluster,
        Gvk::new("", "v1", "Node"),
        node_name.as_str(),
    );

    for initiator in [Initiator::Ui, Initiator::Command, Initiator::Agent] {
        let outcome = w
            .bus
            .dispatch(command(&node), DispatchContext::new(initiator, "someone"))
            .await;
        assert!(
            matches!(outcome, Err(DispatchError::ReadOnly { .. })),
            "{initiator}: {outcome:?}"
        );
    }
    // And no code path around the guard opens one: the service wants the guard's permit.
    let Err(error) = w.service.open_node_shell(&node).await else {
        panic!("nothing allowed this shell");
    };
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert!(pods(&client, ns.name()).await.is_empty());
    assert!(w.opened.lock().is_empty());
}
