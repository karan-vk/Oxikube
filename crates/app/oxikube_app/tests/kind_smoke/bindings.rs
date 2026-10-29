//! RoleBindings and ClusterRoleBindings (E17-S06) against the real adapter: `binding::Create`,
//! `binding::Edit` and `binding::Delete` through the `CommandBus` and `MutationGuard` on a kind
//! cluster, no UI.
//!
//! * a RoleBinding to a ClusterRole is created (a User, a Group and a ServiceAccount subject),
//!   its subjects edited in place at the version read (a stale one is a `Conflict`);
//! * a replace that changes `roleRef` is refused by the API server itself, which is why the
//!   dialog's edit is a delete and a create: `binding::Edit` with `previous` does both as one
//!   audited intent and the new binding names the new role;
//! * a dispatch that is itself a dry run leaves nothing behind, a delete removes the binding;
//! * a ClusterRoleBinding makes the same round trip (its name carries the test namespace's and is
//!   deleted when the test ends, also when it fails).
//!
//! Every RoleBinding lives in the test's own `oxi-test-<rand>` namespace.

use std::sync::Arc;

use k8s_openapi::api::rbac::v1::{ClusterRoleBinding, RoleBinding};
use kube::Api;
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{CommandBus, MutationGuard, binding_ops};
use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use serde_json::{Value, json};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

const GROUP: &str = "rbac.authorization.k8s.io";

/// Deletes a ClusterRoleBinding when dropped (also on panic), best effort: a test that fails half
/// way must not leave one on the shared cluster.
struct ClusterBindingGuard {
    context: String,
    name: String,
}

impl Drop for ClusterBindingGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("kubectl")
            .arg("--context")
            .arg(&self.context)
            .args([
                "delete",
                "clusterrolebinding",
                "--ignore-not-found",
                "--wait=false",
            ])
            .arg(&self.name)
            .output();
    }
}

fn role_binding(namespace: &str, role: &str, version: Option<&str>, subjects: Value) -> Value {
    let mut metadata = json!({"name": "readers", "namespace": namespace});
    if let Some(version) = version {
        metadata["resourceVersion"] = json!(version);
    }
    json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "RoleBinding", "metadata": metadata,
        "roleRef": {"apiGroup": GROUP, "kind": "ClusterRole", "name": role},
        "subjects": subjects,
    })
}

fn subjects(namespace: &str) -> Value {
    json!([
        {"apiGroup": GROUP, "kind": "User", "name": "alice"},
        {"apiGroup": GROUP, "kind": "Group", "name": "fx-team"},
        {"kind": "ServiceAccount", "name": "default", "namespace": namespace},
    ])
}

async fn run(
    bus: &CommandBus,
    command: Command,
    ctx: DispatchContext,
) -> Result<Outcome, DispatchError> {
    // A request the server refuses to dry-run is refused before anything is asked (E19-S07).
    let Outcome::NeedsConfirmation(request) = bus.dispatch(command.clone(), ctx.clone()).await?
    else {
        panic!("expected a confirmation request");
    };
    assert_eq!(
        request.risk,
        Some(oxikube_domain::safety::Risk::Medium),
        "a binding is risk medium"
    );
    bus.dispatch(
        command,
        ctx.with_confirmation(Confirmation::simple(request.token)),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bindings_round_trip_role_ref_recreate_dry_run_and_cluster_bindings() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin_client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
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
        .install("oxikube_app::binding_ops", binding_ops::register_commands)
        .expect("register the binding commands");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
    );
    let ctx = || DispatchContext::new(Initiator::Ui, "tester").with_cluster(entry.cluster.clone());
    let bindings = Api::<RoleBinding>::namespaced(admin_client.clone(), ns.name());
    let cluster = entry.cluster.clone();
    let create = |manifest: &Value| Command::BindingCreate {
        cluster: cluster.clone(),
        manifest: manifest.to_string(),
    };
    let edit = |manifest: &Value, previous: Option<&Value>| Command::BindingEdit {
        cluster: cluster.clone(),
        manifest: manifest.to_string(),
        previous: previous.map(Value::to_string),
    };

    // --- create ----------------------------------------------------------------------------
    run(
        &bus,
        create(&role_binding(ns.name(), "view", None, subjects(ns.name()))),
        ctx(),
    )
    .await
    .expect("create");
    let live = bindings.get("readers").await.expect("the binding exists");
    assert_eq!(live.role_ref.name, "view");
    assert_eq!(live.subjects.as_ref().map(Vec::len), Some(3));
    let version = live.metadata.resource_version.clone().expect("a version");
    let first_uid = live.metadata.uid.clone().expect("a uid");

    // --- edit the subjects in place at the version read; a stale version is a conflict -----
    let fewer = json!([{"apiGroup": GROUP, "kind": "User", "name": "alice"}]);
    run(
        &bus,
        edit(
            &role_binding(ns.name(), "view", Some(&version), fewer.clone()),
            None,
        ),
        ctx(),
    )
    .await
    .expect("edit at the read version");
    let edited = bindings.get("readers").await.expect("get");
    assert_eq!(edited.subjects.as_ref().map(Vec::len), Some(1));
    assert_eq!(edited.metadata.uid.as_deref(), Some(first_uid.as_str()));
    let stale = run(
        &bus,
        edit(
            &role_binding(ns.name(), "view", Some(&version), subjects(ns.name())),
            None,
        ),
        ctx(),
    )
    .await;
    let Err(DispatchError::Handler(error)) = stale else {
        panic!("a stale edit must fail: {stale:?}");
    };
    assert_eq!(error.kind(), ErrorKind::Conflict, "{error:?}");

    // --- the API refuses to change a roleRef in place ---------------------------------------
    let current = bindings.get("readers").await.expect("get");
    let version = current
        .metadata
        .resource_version
        .clone()
        .expect("a version");
    let in_place = run(
        &bus,
        edit(
            &role_binding(ns.name(), "edit", Some(&version), fewer.clone()),
            None,
        ),
        ctx(),
    )
    .await;
    assert!(
        in_place.is_err(),
        "the server refuses a changed roleRef: {in_place:?}"
    );
    assert_eq!(
        bindings.get("readers").await.expect("get").role_ref.name,
        "view"
    );

    // --- so a new roleRef is a delete and a create, as one audited intent -------------------
    let old = role_binding(ns.name(), "view", Some(&version), fewer.clone());
    let new = role_binding(ns.name(), "edit", Some(&version), fewer.clone());
    let audited_before = state.audit_log().len();
    run(&bus, edit(&new, Some(&old)), ctx())
        .await
        .expect("recreate with a new roleRef");
    let recreated = bindings.get("readers").await.expect("get");
    assert_eq!(recreated.role_ref.name, "edit");
    assert_ne!(
        recreated.metadata.uid.as_deref(),
        Some(first_uid.as_str()),
        "a new object"
    );
    let recreate_records: Vec<_> = state.audit_log()[audited_before..]
        .iter()
        // The commit's record: the guard's dry run before it asks (E19-S07) is its own.
        .filter(|r| {
            &*r.cmd == "binding::Edit" && r.outcome == AuditOutcome::Succeeded && !r.dry_run
        })
        .cloned()
        .collect();
    assert_eq!(recreate_records.len(), 1, "one intent, one record");
    assert_eq!(
        recreate_records[0].detail.as_deref(),
        Some("role=ClusterRole/edit subjects=1 recreate=delete+create")
    );

    // --- a dry run leaves nothing behind ----------------------------------------------------
    let mut dry = role_binding(ns.name(), "view", None, fewer);
    dry["metadata"]["name"] = json!("dry");
    run(&bus, create(&dry), ctx().with_dry_run(true))
        .await
        .expect("dry run");
    assert!(bindings.get_opt("dry").await.expect("get").is_none());

    // --- delete ------------------------------------------------------------------------------
    run(
        &bus,
        Command::BindingDelete {
            target: ResourceRef::namespaced(
                cluster.clone(),
                Gvk::new(GROUP, "v1", "RoleBinding"),
                ns.name(),
                "readers",
            ),
        },
        ctx(),
    )
    .await
    .expect("delete");
    assert!(bindings.get_opt("readers").await.expect("get").is_none());

    // --- a ClusterRoleBinding makes the same round trip ---------------------------------------
    let name = format!("{}-readers", ns.name());
    let _cleanup = ClusterBindingGuard {
        context: kind.context.to_string(),
        name: name.clone(),
    };
    let cluster_bindings = Api::<ClusterRoleBinding>::all(admin_client.clone());
    let manifest = |role: &str, version: Option<&str>| {
        let mut metadata = json!({"name": name});
        if let Some(version) = version {
            metadata["resourceVersion"] = json!(version);
        }
        json!({
            "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRoleBinding",
            "metadata": metadata,
            "roleRef": {"apiGroup": GROUP, "kind": "ClusterRole", "name": role},
            "subjects": [{"apiGroup": GROUP, "kind": "Group", "name": "fx-team"}],
        })
    };
    run(&bus, create(&manifest("view", None)), ctx())
        .await
        .expect("create a ClusterRoleBinding");
    let live = cluster_bindings.get(&name).await.expect("exists");
    let old = manifest("view", live.metadata.resource_version.as_deref());
    run(&bus, edit(&manifest("edit", None), Some(&old)), ctx())
        .await
        .expect("recreate the ClusterRoleBinding");
    assert_eq!(
        cluster_bindings
            .get(&name)
            .await
            .expect("get")
            .role_ref
            .name,
        "edit"
    );
    run(
        &bus,
        Command::BindingDelete {
            target: ResourceRef::cluster_scoped(
                cluster.clone(),
                Gvk::new(GROUP, "v1", "ClusterRoleBinding"),
                name.as_str(),
            ),
        },
        ctx(),
    )
    .await
    .expect("delete the ClusterRoleBinding");
    assert!(
        cluster_bindings
            .get_opt(&name)
            .await
            .expect("get")
            .is_none()
    );
}
