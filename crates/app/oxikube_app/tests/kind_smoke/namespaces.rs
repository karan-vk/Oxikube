//! Namespaces (E12-S16) against the real adapter: `namespace::Create` and `namespace::Delete`
//! through the `CommandBus` and `MutationGuard` on a kind cluster, no UI.
//!
//! * a create makes the namespace with its labels (the server dry run and the create are accepted
//!   by the API server) and a second create of the name is a `Conflict` that creates nothing;
//! * a delete through `DeleteFlow` needs the typed name, the namespace really goes (`Terminating`
//!   first or gone at once), and the audit record says what it held;
//! * a read-only session refuses both and the cluster is untouched.
//!
//! The only objects are two cluster-scoped namespaces with a per-run unique name, created and
//! deleted by the test itself: never `default`, `kube-system` or another suite's namespace.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use k8s_openapi::api::core::v1::Namespace;
use kube::Api;
use oxikube_app::command_bus::CommandRegistry;
use oxikube_app::namespaces::{NamespaceCreator, delete_command, register_commands};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{CommandBus, DeleteFlow, ItemStatus, MutationGuard};
use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Propagation;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::safety::ConfirmTier;
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// A name no other run or suite uses.
fn unique(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "{prefix}-{:x}-{:x}",
        std::process::id(),
        nanos & 0xffff_ffff
    )
}

async fn get(admin: &kube::Client, name: &str) -> Option<Namespace> {
    Api::<Namespace>::all(admin.clone())
        .get_opt(name)
        .await
        .expect("get a namespace")
}

fn target(cluster: &ClusterId, name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(cluster.clone(), Gvk::new("", "v1", "Namespace"), name)
}

/// Removes the test's namespace even when an assertion failed first (best effort).
struct Cleanup {
    admin: kube::Client,
    names: Vec<String>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let admin = self.admin.clone();
        let names = std::mem::take(&mut self.names);
        // Deleting is asynchronous on the server: a request is enough, nothing waits for it.
        std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                let api = Api::<Namespace>::all(admin);
                for name in names {
                    let _ = api.delete(&name, &kube::api::DeleteParams::default()).await;
                }
            });
        });
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_delete_read_only_and_conflict_on_a_real_cluster() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin_client = kind.admin_client().await;
    let name = unique("oxi-s16");
    let _cleanup = Cleanup {
        admin: admin_client.clone(),
        names: vec![name.clone()],
    };

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
        .install("oxikube_app::namespaces", register_commands)
        .expect("register the namespace commands");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
    );
    let creator = NamespaceCreator::new(bus.clone(), "tester");
    let cluster = entry.cluster.clone();

    // --- read-only: refused, nothing created ---------------------------------------------------
    manager.set_read_only(&cluster, true).unwrap();
    let err = creator
        .create(&cluster, &name, BTreeMap::new())
        .await
        .expect_err("read-only refuses a create");
    assert_eq!(err.kind(), ErrorKind::Forbidden, "{err}");
    assert!(get(&admin_client, &name).await.is_none());
    manager.set_read_only(&cluster, false).unwrap();

    // --- create with labels ---------------------------------------------------------------------
    let labels = BTreeMap::from([
        ("oxikube.io/test".to_owned(), "s16".to_owned()),
        ("team".to_owned(), "a".to_owned()),
    ]);
    let message = creator
        .create(&cluster, &name, labels.clone())
        .await
        .expect("create");
    assert_eq!(message, format!("Created namespace {name}"));
    let created = get(&admin_client, &name).await.expect("it exists now");
    let have = created.metadata.labels.clone().unwrap_or_default();
    for (key, value) in &labels {
        assert_eq!(have.get(key), Some(value), "label {key}");
    }

    // --- the same name again is a Conflict from the server's dry run ----------------------------
    let err = creator
        .create(&cluster, &name, BTreeMap::new())
        .await
        .expect_err("a second create conflicts");
    assert_eq!(err.kind(), ErrorKind::Conflict, "{err}");

    // --- delete: the typed name, then it goes; the audit says what it held ----------------------
    let contents = BTreeMap::from([("Pod".to_owned(), 0u32)]);
    let flow = DeleteFlow::new(bus.clone(), manager.clone(), "tester")
        .with_command(delete_command(HashMap::from([(name.clone(), contents)])));
    let plan = flow
        .plan(&[target(&cluster, &name)], Propagation::Background)
        .expect("a plan");
    assert_eq!(plan.tier(), ConfirmTier::TypeName);
    assert_eq!(plan.phrase(), Some(name.as_str()));
    flow.run(&plan, Some("not-the-name"))
        .await
        .expect_err("a wrong name sends nothing");
    assert!(get(&admin_client, &name).await.is_some());

    manager.set_read_only(&cluster, true).unwrap();
    let report = flow.run(&plan, Some(&name)).await.expect("run");
    assert!(
        matches!(report.items[0].status, ItemStatus::Forbidden(_)),
        "{report:?}"
    );
    assert!(
        get(&admin_client, &name).await.is_some(),
        "read-only left it alone"
    );
    manager.set_read_only(&cluster, false).unwrap();

    let report = flow.run(&plan, Some(&name)).await.expect("run");
    assert!(report.items[0].status.is_success(), "{report:?}");
    // A namespace is deleted asynchronously: Terminating for a moment, then gone.
    crate::eventually("the namespace to be gone", String::new, || async {
        get(&admin_client, &name).await.is_none()
    })
    .await;

    let audit = state.audit_log();
    let (rehearsed, deleted): (Vec<_>, Vec<_>) = audit
        .iter()
        .filter(|r| &*r.cmd == "namespace::Delete" && r.outcome == AuditOutcome::Succeeded)
        .partition(|r| r.dry_run);
    assert_eq!(deleted.len(), 1, "{audit:?}");
    assert_eq!(
        rehearsed.len(),
        1,
        "the guard dry-ran the irreversible delete before asking: {audit:?}"
    );
    assert_eq!(deleted[0].initiator, Initiator::Ui);
    assert_eq!(deleted[0].detail.as_deref(), Some("contents=Pod:0"));
    assert_eq!(&*deleted[0].target.name, name);
    let created: Vec<_> = audit
        .iter()
        // The commit's record: the guard's dry run before it asks (E19-S07) is its own.
        .filter(|r| {
            &*r.cmd == "namespace::Create" && r.outcome == AuditOutcome::Succeeded && !r.dry_run
        })
        .collect();
    assert_eq!(created.len(), 1, "{audit:?}");
    assert!(
        audit
            .iter()
            .any(|r| r.outcome == AuditOutcome::Denied && &*r.cmd == "namespace::Create"),
        "the read-only refusal is audited: {audit:?}"
    );
}
