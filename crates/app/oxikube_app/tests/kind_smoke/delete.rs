//! Row actions and delete (E07-S08) against the real adapter: `resource::Delete` through the
//! `CommandBus` and `MutationGuard` on a kind cluster, no UI.
//!
//! * a selection of ConfigMaps (one already gone) deletes with one result per object and one
//!   audit record each; the objects really leave the cluster, the server dry run and the
//!   propagation policy are accepted by the API server;
//! * a dispatch that is itself a dry run changes nothing;
//! * a read-only session refuses every object and the cluster is untouched;
//! * a user who may not delete (a viewer service account) is not offered delete, and the
//!   server's own refusal is the item's `Forbidden`.
//!
//! Every object lives in the test's own `oxi-test-<rand>` namespace.

use std::sync::Arc;

use k8s_openapi::api::core::v1::ConfigMap;
use kube::Api;
use kube::api::{ObjectMeta, PostParams};
use oxikube_app::actions::register_commands;
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{
    ActionContext, CommandBus, DeleteFlow, ItemStatus, MutationGuard, RowActionRegistry, RowActions,
};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, Propagation};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, Verb, VerbSet};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry, read_only_pod_viewer};

fn config_map_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "ConfigMap"),
        preferred: true,
        plural: "configmaps".into(),
        singular: "configmap".into(),
        short_names: vec!["cm".into()],
        categories: Vec::new(),
        verbs: VerbSet::from([Verb::Get, Verb::List, Verb::Watch, Verb::Delete]),
        namespaced: true,
    }
}

async fn create_config_maps(admin: &kube::Client, namespace: &str, names: &[&str]) {
    let api = Api::<ConfigMap>::namespaced(admin.clone(), namespace);
    for name in names {
        api.create(
            &PostParams::default(),
            &ConfigMap {
                metadata: ObjectMeta {
                    name: Some((*name).to_owned()),
                    namespace: Some(namespace.to_owned()),
                    ..ObjectMeta::default()
                },
                ..ConfigMap::default()
            },
        )
        .await
        .expect("create a config map");
    }
}

async fn exists(admin: &kube::Client, namespace: &str, name: &str) -> bool {
    Api::<ConfigMap>::namespaced(admin.clone(), namespace)
        .get_opt(name)
        .await
        .expect("get")
        .is_some()
}

fn target(cluster: &ClusterId, namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("", "v1", "ConfigMap"),
        namespace,
        name,
    )
}

struct World {
    manager: ClusterSessionManager,
    bus: CommandBus,
    state: Arc<FakeStatePort>,
}

fn world(connector: KubeConnector) -> World {
    let manager = ClusterSessionManager::with_config(
        Arc::new(connector),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
        SessionManagerConfig::default(),
    );
    let state = Arc::new(FakeStatePort::new());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::actions", register_commands)
        .expect("register resource::Delete");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
    );
    World {
        manager,
        bus,
        state,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_selection_dry_run_read_only_and_viewer() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin_client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    create_config_maps(&admin_client, ns.name(), &["keep", "a", "b", "dry", "ro"]).await;

    // The admin, and a service account that may read pods and nothing else.
    let viewer_token = read_only_pod_viewer(&admin_client, ns.name(), "oxi-delete-viewer").await;
    let mut kubeconfig = kind.kubeconfig.clone();
    let viewer_context = ContextName::new(format!("oxi-delete-viewer-{}", ns.name()));
    kind.add_token_context(&mut kubeconfig, viewer_context.as_str(), &viewer_token);
    let (admin, viewer) = (catalog_entry(&kind.context), catalog_entry(&viewer_context));
    let connector = KubeConnector::new(
        kubeconfig,
        PoolConfig::default(),
        ConnectorConfig::default(),
    );
    let w = world(connector);
    w.manager.open(&admin, SessionOptions::default());
    w.manager.open(&viewer, SessionOptions::default());
    w.manager
        .connect(&admin.cluster)
        .await
        .expect("connect admin");
    w.manager
        .connect(&viewer.cluster)
        .await
        .expect("connect viewer");

    // --- who is offered delete ----------------------------------------------------------------
    let actions = RowActions::from_bus(&w.bus, &RowActionRegistry::core());
    let session_of = |entry: &oxikube_ports::ClusterContext| w.manager.get(&entry.cluster).unwrap();
    let admin_ctx = ActionContext::of(&session_of(&admin));
    let viewer_ctx = ActionContext::of(&session_of(&viewer));
    assert_eq!(actions.resolve(&config_map_kind(), &admin_ctx, 1).len(), 1);
    assert!(
        actions
            .actions_for(&config_map_kind(), viewer_ctx.capabilities)
            .is_empty(),
        "a session without MUTATE is not offered delete: {:?}",
        viewer_ctx.capabilities
    );

    // --- a selection: two exist, one is gone already -------------------------------------------
    let flow = DeleteFlow::new(w.bus.clone(), w.manager.clone(), "tester");
    let targets = vec![
        target(&admin.cluster, ns.name(), "a"),
        target(&admin.cluster, ns.name(), "missing"),
        target(&admin.cluster, ns.name(), "b"),
    ];
    let plan = flow.plan(&targets, Propagation::Background).unwrap();
    assert!(plan.phrase().is_none(), "ConfigMaps take a simple confirm");
    let report = flow.run(&plan, None).await.expect("run");
    let statuses: Vec<_> = report.items.iter().map(|i| &i.status).collect();
    assert_eq!(statuses[0], &ItemStatus::Deleted, "{report:?}");
    assert!(matches!(statuses[1], ItemStatus::NotFound(_)), "{report:?}");
    assert_eq!(statuses[2], &ItemStatus::Deleted, "{report:?}");
    assert!(!exists(&admin_client, ns.name(), "a").await);
    assert!(!exists(&admin_client, ns.name(), "b").await);
    assert!(
        exists(&admin_client, ns.name(), "keep").await,
        "only the selection went"
    );
    let audit = w.state.audit_log();
    assert_eq!(audit.len(), 3, "one record per object: {audit:?}");
    assert!(audit.iter().all(|r| r.initiator == Initiator::Ui));
    assert_eq!(
        audit
            .iter()
            .filter(|r| r.outcome == AuditOutcome::Succeeded)
            .count(),
        2
    );

    // --- foreground propagation is accepted by the API server ---------------------------------
    let plan = flow
        .plan(
            &[target(&admin.cluster, ns.name(), "dry")],
            Propagation::Foreground,
        )
        .unwrap();
    assert_eq!(
        plan.phrase(),
        Some("dry"),
        "a cascading delete takes the name"
    );
    let report = flow.run(&plan, Some("dry")).await.expect("run");
    assert!(report.items[0].status.is_success(), "{report:?}");
    // Foreground: the object stays (finalizer) until the garbage collector has run.
    crate::eventually("the foreground delete to finish", String::new, || async {
        !exists(&admin_client, ns.name(), "dry").await
    })
    .await;

    // --- a dispatch that is a dry run changes nothing ------------------------------------------
    create_config_maps(&admin_client, ns.name(), &["dry"]).await;
    let command = Command::ResourceDelete {
        target: target(&admin.cluster, ns.name(), "dry"),
        propagation: Propagation::Background,
    };
    let ctx = DispatchContext::new(Initiator::Command, "tester").with_dry_run(true);
    let Ok(Outcome::NeedsConfirmation(request)) =
        w.bus.dispatch(command.clone(), ctx.clone()).await
    else {
        panic!("expected a confirmation request");
    };
    w.bus
        .dispatch(
            command,
            ctx.with_confirmation(Confirmation::simple(request.token)),
        )
        .await
        .expect("dry run");
    assert!(
        exists(&admin_client, ns.name(), "dry").await,
        "a dry run deletes nothing"
    );

    // --- read-only: refused for every object, the cluster untouched ----------------------------
    w.manager.set_read_only(&admin.cluster, true).unwrap();
    let plan = flow
        .plan(
            &[target(&admin.cluster, ns.name(), "ro")],
            Propagation::Background,
        )
        .unwrap();
    let report = flow.run(&plan, None).await.expect("run");
    assert!(
        matches!(report.items[0].status, ItemStatus::Forbidden(_)),
        "{report:?}"
    );
    assert!(exists(&admin_client, ns.name(), "ro").await);
    w.manager.set_read_only(&admin.cluster, false).unwrap();

    // --- the viewer: if a stale menu dispatched anyway, the server's refusal is Forbidden ------
    let plan = flow
        .plan(
            &[target(&viewer.cluster, ns.name(), "ro")],
            Propagation::Background,
        )
        .unwrap();
    let report = flow.run(&plan, None).await.expect("run");
    assert!(
        matches!(report.items[0].status, ItemStatus::Forbidden(_)),
        "{report:?}"
    );
    assert!(exists(&admin_client, ns.name(), "ro").await);
}
