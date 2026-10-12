//! The bulk actions (E12-S19) against the real adapter: `selection::Delete`, `selection::Restart`
//! and `selection::Label` through the `CommandBus` and `MutationGuard` on a kind cluster, no UI.
//!
//! * a selection of ConfigMaps, a Deployment and a StatefulSet (mixed kinds, one object already
//!   gone) gets its labels added, changed and removed (a merge patch with `null`) and an
//!   annotation set, one result per object and one audit record each under a shared batch id;
//! * Deployments and StatefulSets restart in bulk (the pod template is stamped), and a selection
//!   with a ConfigMap in it is refused before a request is sent;
//! * a bulk delete of ConfigMaps (one gone) and a throwaway Namespace takes the typed cluster name
//!   (the highest tier among the objects), deletes the lot, and a wrong phrase sends nothing;
//! * a read-only session refuses the whole batch and the cluster is untouched.
//!
//! Every object lives in the test's own `oxi-test-<rand>` namespace, the throwaway Namespace has a
//! unique name and holds nothing, and nothing touches the node. The Deployment and the
//! StatefulSet have no replicas, so no pod is ever scheduled.

use std::sync::Arc;

use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{ConfigMap, Namespace};
use kube::Api;
use kube::api::{ObjectMeta, PostParams};
use oxikube_app::command_bus::CommandRegistry;
use oxikube_app::selection::register_commands;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{BulkReport, CommandBus, MutationGuard, SelectionError, SelectionFlow};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, Propagation};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::safety::ConfirmTier;
use oxikube_domain::selection::{LabelEdit, LabelField};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::images::PAUSE;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use serde_json::json;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";

struct Rig {
    flow: SelectionFlow,
    manager: ClusterSessionManager,
    state: Arc<FakeStatePort>,
    cluster: ClusterId,
}

impl Rig {
    async fn new(kind: &Kind) -> Self {
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
        let state = Arc::new(FakeStatePort::new());
        let mut registry = CommandRegistry::new();
        let handler_sessions = manager.clone();
        registry
            .install("oxikube_app::selection", move |r| {
                register_commands(r, handler_sessions)
            })
            .expect("register the bulk commands");
        let bus = CommandBus::new(
            registry,
            MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
        );
        manager.open(&entry, SessionOptions::default());
        manager.connect(&entry.cluster).await.expect("connect");
        Self {
            flow: SelectionFlow::new(bus, manager.clone(), "tester"),
            manager,
            state,
            cluster: entry.cluster,
        }
    }

    fn target(&self, group: &str, kind: &str, namespace: &str, name: &str) -> ResourceRef {
        ResourceRef::namespaced(
            self.cluster.clone(),
            Gvk::new(group, "v1", kind),
            namespace,
            name,
        )
    }

    /// Plans and runs `command`, typing the phrase when the plan asks for one.
    async fn run(&self, command: Command) -> Result<BulkReport, SelectionError> {
        let plan = self.flow.plan(command)?;
        let phrase = plan.phrase().map(str::to_owned);
        self.flow.run(&plan, phrase.as_deref()).await
    }
}

fn config_map(namespace: &str, name: &str) -> ConfigMap {
    ConfigMap {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some(namespace.to_owned()),
            labels: Some(
                [
                    ("app".to_owned(), "demo".to_owned()),
                    ("old".to_owned(), "x".to_owned()),
                ]
                .into(),
            ),
            ..ObjectMeta::default()
        },
        ..ConfigMap::default()
    }
}

fn template(app: &str) -> serde_json::Value {
    json!({"metadata": {"labels": {"app": app}},
           "spec": {"terminationGracePeriodSeconds": 0,
                    "containers": [{"name": "main", "image": PAUSE}]}})
}

fn deployment(name: &str) -> Deployment {
    serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": name},
        "spec": {"replicas": 0, "selector": {"matchLabels": {"app": name}}, "template": template(name)}
    }))
    .expect("a deployment")
}

fn statefulset(name: &str) -> StatefulSet {
    serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "StatefulSet",
        "metadata": {"name": name},
        "spec": {"replicas": 0, "serviceName": name,
                 "selector": {"matchLabels": {"app": name}}, "template": template(name)}
    }))
    .expect("a statefulset")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn labels_and_annotations_change_on_mixed_kinds_in_one_batch() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let maps = Api::<ConfigMap>::namespaced(admin.clone(), ns.name());
    for name in ["a", "b"] {
        maps.create(&PostParams::default(), &config_map(ns.name(), name))
            .await
            .expect("create");
    }
    let deployments = Api::<Deployment>::namespaced(admin.clone(), ns.name());
    deployments
        .create(&PostParams::default(), &deployment("web"))
        .await
        .expect("create the deployment");
    let rig = Rig::new(&kind).await;

    let targets = vec![
        rig.target("", "ConfigMap", ns.name(), "a"),
        rig.target("", "ConfigMap", ns.name(), "ghost"),
        rig.target("apps", "Deployment", ns.name(), "web"),
        rig.target("", "ConfigMap", ns.name(), "b"),
    ];
    let report = rig
        .run(Command::SelectionLabel {
            targets,
            field: LabelField::Labels,
            edits: vec![
                LabelEdit::set("app", "changed"),
                LabelEdit::set("team", "data"),
                LabelEdit::remove("old"),
            ],
        })
        .await
        .expect("the batch ran");
    let status: Vec<_> = report.rows.iter().map(|r| r.status.label()).collect();
    assert_eq!(status, ["Done", "Skipped", "Done", "Done"], "{report:?}");
    let first_batch = report.batch.clone();

    for name in ["a", "b"] {
        let labels = maps
            .get(name)
            .await
            .unwrap()
            .metadata
            .labels
            .unwrap_or_default();
        assert_eq!(
            labels.get("app").map(String::as_str),
            Some("changed"),
            "{name}"
        );
        assert_eq!(
            labels.get("team").map(String::as_str),
            Some("data"),
            "{name}"
        );
        assert!(
            !labels.contains_key("old"),
            "{name}: removed with a null in the merge patch"
        );
    }
    let web = deployments
        .get("web")
        .await
        .unwrap()
        .metadata
        .labels
        .unwrap_or_default();
    assert_eq!(web.get("team").map(String::as_str), Some("data"));

    // The same command with the other field sets an annotation (free text) and leaves labels be.
    let report = rig
        .run(Command::SelectionLabel {
            targets: vec![rig.target("", "ConfigMap", ns.name(), "a")],
            field: LabelField::Annotations,
            edits: vec![LabelEdit::set("example.io/note", "free text: with spaces")],
        })
        .await
        .expect("annotate");
    assert_eq!(report.succeeded(), 1);
    let a = maps.get("a").await.unwrap().metadata;
    assert_eq!(
        a.annotations
            .unwrap_or_default()
            .get("example.io/note")
            .map(String::as_str),
        Some("free text: with spaces")
    );
    assert_eq!(
        a.labels.unwrap_or_default().get("team").map(String::as_str),
        Some("data")
    );

    // A record per object (the ghost's is a success: gone is not a failure) under one batch id,
    // none carrying a value.
    let audit = rig.state.audit_log();
    let per_object: Vec<_> = audit
        .iter()
        .filter(|r| {
            &*r.cmd == "selection::Label"
                && r.detail.as_deref().is_some_and(|d| {
                    d.starts_with(&format!("batch={first_batch} via=selection::Label"))
                })
        })
        .collect();
    assert_eq!(per_object.len(), 4, "{audit:?}");
    assert!(audit.iter().all(|r| r.initiator == Initiator::Ui));
    assert!(audit.iter().all(|r| !format!("{r:?}").contains("changed")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deployments_and_statefulsets_restart_in_bulk_and_a_configmap_refuses_the_batch() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let deployments = Api::<Deployment>::namespaced(admin.clone(), ns.name());
    let statefulsets = Api::<StatefulSet>::namespaced(admin.clone(), ns.name());
    let maps = Api::<ConfigMap>::namespaced(admin.clone(), ns.name());
    deployments
        .create(&PostParams::default(), &deployment("web"))
        .await
        .expect("create the deployment");
    statefulsets
        .create(&PostParams::default(), &statefulset("db"))
        .await
        .expect("create the statefulset");
    maps.create(&PostParams::default(), &config_map(ns.name(), "cfg"))
        .await
        .expect("create");
    let rig = Rig::new(&kind).await;
    let web = rig.target("apps", "Deployment", ns.name(), "web");
    let db = rig.target("apps", "StatefulSet", ns.name(), "db");

    // A ConfigMap in the selection refuses the whole batch before a request: nothing is stamped.
    let refused = rig
        .run(Command::SelectionRestart {
            targets: vec![web.clone(), rig.target("", "ConfigMap", ns.name(), "cfg")],
        })
        .await;
    match refused {
        Err(SelectionError::Refused(error)) => {
            assert_eq!(error.kind(), oxikube_domain::ErrorKind::Validation);
        }
        other => panic!("{other:?}"),
    }
    let stamp = |template: &k8s_openapi::api::core::v1::PodTemplateSpec| {
        template
            .metadata
            .as_ref()
            .and_then(|m| m.annotations.as_ref())
            .and_then(|a| a.get(RESTARTED_AT))
            .cloned()
    };
    assert_eq!(
        stamp(&deployments.get("web").await.unwrap().spec.unwrap().template),
        None
    );

    let report = rig
        .run(Command::SelectionRestart {
            targets: vec![web, db],
        })
        .await
        .expect("restart");
    assert_eq!((report.succeeded(), report.failed()), (2, 0), "{report:?}");
    assert!(stamp(&deployments.get("web").await.unwrap().spec.unwrap().template).is_some());
    assert!(stamp(&statefulsets.get("db").await.unwrap().spec.unwrap().template).is_some());
    let restarts: Vec<_> = rig
        .state
        .audit_log()
        .into_iter()
        .filter(|r| &*r.cmd == "workload::Restart")
        .collect();
    assert_eq!(restarts.len(), 2);
    assert!(
        restarts
            .iter()
            .all(|r| r.outcome == AuditOutcome::Succeeded)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_namespace_in_the_selection_makes_the_batch_type_the_cluster_name() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let maps = Api::<ConfigMap>::namespaced(admin.clone(), ns.name());
    for name in ["x", "y"] {
        maps.create(&PostParams::default(), &config_map(ns.name(), name))
            .await
            .expect("create");
    }
    // A throwaway Namespace of its own: unique, empty, deleted by the batch.
    let throwaway = format!("oxi-bulk-{}", ns.name().trim_start_matches("oxi-test-"));
    let namespaces = Api::<Namespace>::all(admin.clone());
    namespaces
        .create(
            &PostParams::default(),
            &Namespace {
                metadata: ObjectMeta {
                    name: Some(throwaway.clone()),
                    ..ObjectMeta::default()
                },
                ..Namespace::default()
            },
        )
        .await
        .expect("create the throwaway namespace");
    let rig = Rig::new(&kind).await;
    let targets = vec![
        rig.target("", "ConfigMap", ns.name(), "x"),
        rig.target("", "ConfigMap", ns.name(), "gone-already"),
        rig.target("", "ConfigMap", ns.name(), "y"),
        ResourceRef::cluster_scoped(
            rig.cluster.clone(),
            Gvk::new("", "v1", "Namespace"),
            throwaway.clone(),
        ),
    ];
    let command = Command::SelectionDelete {
        targets: targets.clone(),
        propagation: Propagation::Background,
    };
    let plan = rig.flow.plan(command).unwrap();
    assert_eq!(
        plan.tier(),
        ConfirmTier::TypeName,
        "one Namespace raises the whole batch"
    );
    let context = rig.manager.get(&rig.cluster).unwrap().context().to_string();
    assert_eq!(plan.phrase(), Some(context.as_str()));

    // A wrong phrase sends nothing.
    let wrong = rig.flow.run(&plan, Some("not-the-cluster")).await;
    assert!(matches!(wrong, Err(SelectionError::NameMismatch { .. })));
    assert!(maps.get_opt("x").await.unwrap().is_some());

    let report = rig
        .flow
        .run(&plan, Some(&context))
        .await
        .expect("the batch ran");
    let status: Vec<_> = report.rows.iter().map(|r| r.status.label()).collect();
    assert_eq!(status, ["Done", "Skipped", "Done", "Done"], "{report:?}");
    assert!(maps.get_opt("x").await.unwrap().is_none());
    assert!(maps.get_opt("y").await.unwrap().is_none());
    // The namespace is terminating (or gone): its deletion timestamp is set, or it 404s.
    let left = namespaces.get_opt(&throwaway).await.unwrap();
    assert!(left.is_none_or(|n| n.metadata.deletion_timestamp.is_some()));

    let audit = rig.state.audit_log();
    assert_eq!(
        audit
            .iter()
            .filter(
                |r| matches!(&*r.cmd, "resource::Delete") && r.outcome == AuditOutcome::Succeeded
            )
            .count(),
        4,
        "a record per object: {audit:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_only_session_refuses_the_whole_batch_and_the_cluster_is_untouched() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let maps = Api::<ConfigMap>::namespaced(admin.clone(), ns.name());
    maps.create(&PostParams::default(), &config_map(ns.name(), "keep"))
        .await
        .expect("create");
    let rig = Rig::new(&kind).await;
    rig.manager
        .set_read_only(&rig.cluster, true)
        .expect("read-only");
    for command in [
        Command::SelectionDelete {
            targets: vec![rig.target("", "ConfigMap", ns.name(), "keep")],
            propagation: Propagation::Background,
        },
        Command::SelectionLabel {
            targets: vec![rig.target("", "ConfigMap", ns.name(), "keep")],
            field: LabelField::Labels,
            edits: vec![LabelEdit::set("a", "b")],
        },
    ] {
        match rig.run(command).await {
            Err(SelectionError::Refused(error)) => {
                assert_eq!(error.kind(), oxikube_domain::ErrorKind::Forbidden)
            }
            other => panic!("{other:?}"),
        }
    }
    let live = maps.get("keep").await.unwrap();
    assert_eq!(
        live.metadata
            .labels
            .unwrap_or_default()
            .get("app")
            .map(String::as_str),
        Some("demo"),
        "untouched"
    );
    assert!(
        rig.state
            .audit_log()
            .iter()
            .all(|r| r.outcome == AuditOutcome::Denied)
    );
}
