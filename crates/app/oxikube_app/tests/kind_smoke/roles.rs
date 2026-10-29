//! Roles and ClusterRoles (E17-S05) against the real adapter: `role::Create`, `role::Edit` and
//! `role::Delete` through the `CommandBus` and `MutationGuard` on a kind cluster, no UI.
//!
//! * a Role is created, edited (a replace at the version the dialog read: a stale one is a
//!   `Conflict` and changes nothing), counted by the bindings that name it and deleted;
//! * a dispatch that is itself a dry run leaves nothing behind;
//! * the server's own validation refuses a rule the dialog's checks let through;
//! * a ClusterRole makes the same round trip (its name carries the test namespace's and is
//!   deleted when the test ends, also when it fails).
//!
//! Every Role lives in the test's own `oxi-test-<rand>` namespace.

use std::sync::Arc;

use k8s_openapi::api::rbac::v1::{ClusterRole, Role, RoleBinding, RoleRef, Subject};
use kube::Api;
use kube::api::{ObjectMeta, PostParams};
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::role_ops::{RoleLinks, register_commands};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{CommandBus, MutationGuard};
use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::rbac::RoleId;
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use serde_json::{Value, json};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// Deletes a cluster-scoped ClusterRole when dropped (also on panic), best effort: a test that
/// fails half way must not leave one on the shared cluster.
struct ClusterRoleGuard {
    context: String,
    name: String,
}

impl Drop for ClusterRoleGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("kubectl")
            .arg("--context")
            .arg(&self.context)
            .args([
                "delete",
                "clusterrole",
                "--ignore-not-found",
                "--wait=false",
            ])
            .arg(&self.name)
            .output();
    }
}

fn role_json(namespace: &str, name: &str, version: Option<&str>, verbs: &[&str]) -> Value {
    let mut metadata = json!({"name": name, "namespace": namespace});
    if let Some(version) = version {
        metadata["resourceVersion"] = json!(version);
    }
    json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role", "metadata": metadata,
        "rules": [{"apiGroups": [""], "resources": ["configmaps"], "verbs": verbs}],
    })
}

fn cluster_role_json(name: &str, version: Option<&str>, verbs: &[&str]) -> Value {
    let mut metadata = json!({"name": name});
    if let Some(version) = version {
        metadata["resourceVersion"] = json!(version);
    }
    json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole", "metadata": metadata,
        "rules": [{"apiGroups": [""], "resources": ["configmaps"], "verbs": verbs}],
    })
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
        "an RBAC change is risk medium"
    );
    bus.dispatch(
        command,
        ctx.with_confirmation(Confirmation::simple(request.token)),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn roles_round_trip_conflict_dry_run_and_cluster_roles() {
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
        .install("oxikube_app::role_ops", register_commands)
        .expect("register the role commands");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
    );
    let ctx = || DispatchContext::new(Initiator::Ui, "tester").with_cluster(entry.cluster.clone());
    let roles = Api::<Role>::namespaced(admin_client.clone(), ns.name());
    let cluster = entry.cluster.clone();
    let create = |manifest: &Value| Command::RoleCreate {
        cluster: cluster.clone(),
        manifest: manifest.to_string(),
    };
    let edit = |manifest: &Value| Command::RoleEdit {
        cluster: cluster.clone(),
        manifest: manifest.to_string(),
    };

    // --- create, then edit at the version read -------------------------------------------------
    run(
        &bus,
        create(&role_json(ns.name(), "reader", None, &["get"])),
        ctx(),
    )
    .await
    .expect("create");
    let live = roles.get("reader").await.expect("the Role exists");
    assert_eq!(live.rules.as_ref().map(Vec::len), Some(1));
    let version = live.metadata.resource_version.clone().expect("a version");
    run(
        &bus,
        edit(&role_json(
            ns.name(),
            "reader",
            Some(&version),
            &["get", "list"],
        )),
        ctx(),
    )
    .await
    .expect("edit at the read version");
    let edited = roles.get("reader").await.expect("get");
    let verbs = edited.rules.expect("rules")[0].verbs.clone();
    assert_eq!(verbs, ["get", "list"]);

    // --- a stale version is a conflict and changes nothing -------------------------------------
    let stale = run(
        &bus,
        edit(&role_json(
            ns.name(),
            "reader",
            Some(&version),
            &["get", "list", "delete"],
        )),
        ctx(),
    )
    .await;
    let Err(DispatchError::Handler(error)) = stale else {
        panic!("a stale edit must fail: {stale:?}");
    };
    assert_eq!(error.kind(), ErrorKind::Conflict, "{error:?}");
    let unchanged = roles
        .get("reader")
        .await
        .expect("get")
        .rules
        .expect("rules");
    assert_eq!(unchanged[0].verbs, ["get", "list"]);

    // --- the server's validation refuses what the dialog's checks let through ------------------
    let invalid = json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
        "metadata": {"name": "invalid", "namespace": ns.name()},
        "rules": [{"apiGroups": [""], "resources": ["configmaps"], "verbs": []}],
    });
    assert!(run(&bus, create(&invalid), ctx()).await.is_err());
    assert!(roles.get_opt("invalid").await.expect("get").is_none());

    // --- a dry run leaves nothing behind -------------------------------------------------------
    let dry = role_json(ns.name(), "dry", None, &["get"]);
    run(&bus, create(&dry), ctx().with_dry_run(true))
        .await
        .expect("dry run");
    assert!(roles.get_opt("dry").await.expect("get").is_none());

    // --- bindings that name the role are counted, then the delete goes through -----------------
    let bindings = Api::<RoleBinding>::namespaced(admin_client.clone(), ns.name());
    bindings
        .create(
            &PostParams::default(),
            &RoleBinding {
                metadata: ObjectMeta {
                    name: Some("readers".into()),
                    ..ObjectMeta::default()
                },
                role_ref: RoleRef {
                    api_group: Some("rbac.authorization.k8s.io".into()),
                    kind: "Role".into(),
                    name: "reader".into(),
                },
                subjects: Some(vec![Subject {
                    kind: "User".into(),
                    name: "alice".into(),
                    ..Subject::default()
                }]),
            },
        )
        .await
        .expect("create a binding");
    let links = RoleLinks::new(manager.clone());
    let count = links
        .binding_count(&cluster, &RoleId::role(ns.name(), "reader"))
        .await
        .expect("count");
    assert_eq!(count, 1);
    let target = ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("rbac.authorization.k8s.io", "v1", "Role"),
        ns.name(),
        "reader",
    );
    run(
        &bus,
        Command::RoleDelete {
            target,
            bindings: Some(count),
        },
        ctx(),
    )
    .await
    .expect("delete");
    assert!(roles.get_opt("reader").await.expect("get").is_none());

    // --- a ClusterRole makes the same round trip ----------------------------------------------
    let name = format!("{}-reader", ns.name());
    let _cleanup = ClusterRoleGuard {
        context: kind.context.to_string(),
        name: name.clone(),
    };
    let cluster_roles = Api::<ClusterRole>::all(admin_client.clone());
    run(
        &bus,
        create(&cluster_role_json(&name, None, &["get"])),
        ctx(),
    )
    .await
    .expect("create a ClusterRole");
    let live = cluster_roles.get(&name).await.expect("exists");
    let version = live.metadata.resource_version.expect("a version");
    run(
        &bus,
        edit(&cluster_role_json(&name, Some(&version), &["get", "watch"])),
        ctx(),
    )
    .await
    .expect("edit the ClusterRole");
    assert_eq!(
        cluster_roles
            .get(&name)
            .await
            .expect("get")
            .rules
            .expect("rules")[0]
            .verbs,
        ["get", "watch"]
    );
    run(
        &bus,
        Command::RoleDelete {
            target: ResourceRef::cluster_scoped(
                cluster.clone(),
                Gvk::new("rbac.authorization.k8s.io", "v1", "ClusterRole"),
                name.as_str(),
            ),
            bindings: None,
        },
        ctx(),
    )
    .await
    .expect("delete the ClusterRole");
    assert!(cluster_roles.get_opt(&name).await.expect("get").is_none());

    // --- every attempt was audited, with a rule count and never the rules ---------------------
    let audit = state.audit_log();
    assert!(
        audit.iter().any(|r| r.outcome == AuditOutcome::Failed),
        "{audit:?}"
    );
    assert!(
        audit
            .iter()
            .filter(|r| &*r.cmd == "role::Edit" && r.outcome == AuditOutcome::Succeeded)
            .all(|r| r.detail.as_deref() == Some("rules=1")),
        "{audit:?}"
    );
}
