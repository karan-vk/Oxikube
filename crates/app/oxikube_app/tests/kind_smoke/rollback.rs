//! Rollout history and rollback (E12-S06) against the real adapter: `RolloutService` and
//! `workload::Rollback` through the `CommandBus` and `MutationGuard` on a kind cluster, no UI.
//!
//! * a Deployment whose image is changed twice has three revisions in its history, newest first,
//!   with the change causes that were annotated; rolling back restores the template (image and
//!   labels, no `pod-template-hash`) and the Deployment controller records the restored
//!   ReplicaSet as a new revision (the annotation);
//! * rolling back to a named revision, to the revision already running ("already at"), a dry run,
//!   a paused Deployment (kubectl's refusal) and a read-only session;
//! * a StatefulSet is rolled back through its ControllerRevisions, as a strategic merge patch;
//! * every attempt leaves one audit record with the revisions it went from and to.
//!
//! Every object lives in the test's own `oxi-test-<rand>` namespace; nothing touches the node, and
//! the workloads have no replicas, so no image is pulled.

use std::sync::Arc;

use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use kube::Api;
use kube::api::{Patch, PatchParams, PostParams};
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::workloads::register_commands;
use oxikube_app::{CommandBus, MutationGuard, RolloutService};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort, images};
use serde_json::{Value, json};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

const REVISION: &str = "deployment.kubernetes.io/revision";
const CAUSE: &str = "kubernetes.io/change-cause";

fn labels(name: &str) -> Value {
    json!({"app": name})
}

fn template(name: &str, image: &str) -> Value {
    json!({
        "metadata": {"labels": labels(name)},
        "spec": {
            "terminationGracePeriodSeconds": 0,
            "containers": [{"name": "app", "image": image, "command": ["sleep", "3600"]}]
        }
    })
}

fn deployment(name: &str, image: &str) -> Deployment {
    serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": name, "annotations": {CAUSE: "first image"}},
        "spec": {
            "replicas": 0,
            "selector": {"matchLabels": labels(name)},
            "template": template(name, image),
        }
    }))
    .expect("a deployment")
}

fn statefulset(name: &str, image: &str) -> StatefulSet {
    serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "StatefulSet",
        "metadata": {"name": name},
        "spec": {
            "replicas": 0,
            "serviceName": name,
            "selector": {"matchLabels": labels(name)},
            "template": template(name, image),
        }
    }))
    .expect("a statefulset")
}

/// Sets the image of container `app` (and the change cause) the way `kubectl set image` would.
async fn set_image<K>(api: &Api<K>, name: &str, image: &str, cause: Option<&str>)
where
    K: Clone + serde::de::DeserializeOwned + std::fmt::Debug,
{
    let mut patch = json!({"spec": {"template": {"spec": {"containers": [
        {"name": "app", "image": image}
    ]}}}});
    if let Some(cause) = cause {
        patch["metadata"] = json!({"annotations": {CAUSE: cause}});
    }
    api.patch(name, &PatchParams::default(), &Patch::Strategic(patch))
        .await
        .expect("set the image");
}

async fn image_of(api: &Api<Deployment>, name: &str) -> String {
    api.get(name)
        .await
        .expect("get")
        .spec
        .and_then(|spec| spec.template.spec)
        .map(|spec| spec.containers[0].image.clone().unwrap_or_default())
        .unwrap_or_default()
}

async fn revision_of(api: &Api<Deployment>, name: &str) -> String {
    api.get(name)
        .await
        .expect("get")
        .metadata
        .annotations
        .and_then(|annotations| annotations.get(REVISION).cloned())
        .unwrap_or_default()
}

struct Rig {
    manager: ClusterSessionManager,
    bus: CommandBus,
    state: Arc<FakeStatePort>,
    cluster: ClusterId,
}

impl Rig {
    async fn open(kind: &Kind) -> Self {
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
            .install("oxikube_app::workloads", move |r| {
                register_commands(r, handler_sessions)
            })
            .expect("register workload::Rollback");
        let bus = CommandBus::new(
            registry,
            MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
        );
        manager.open(&entry, SessionOptions::default());
        manager.connect(&entry.cluster).await.expect("connect");
        Self {
            manager,
            bus,
            state,
            cluster: entry.cluster,
        }
    }

    fn target(&self, kind: &str, namespace: &str, name: &str) -> ResourceRef {
        ResourceRef::namespaced(
            self.cluster.clone(),
            Gvk::new("apps", "v1", kind),
            namespace,
            name,
        )
    }

    fn ctx(&self) -> DispatchContext {
        DispatchContext::new(Initiator::Ui, "tester").with_cluster(self.cluster.clone())
    }

    /// Dispatches `command` and answers the guard's confirmation, as the dialog does.
    async fn run(&self, command: Command, ctx: DispatchContext) -> Result<Outcome, DispatchError> {
        // A request the server refuses to dry-run is refused before anything is asked
        // (E19-S07).
        let Outcome::NeedsConfirmation(asked) =
            self.bus.dispatch(command.clone(), ctx.clone()).await?
        else {
            panic!("a rollback asks for a confirmation");
        };
        self.bus
            .dispatch(
                command,
                ctx.with_confirmation(Confirmation::simple(asked.token)),
            )
            .await
    }

    async fn rollback(&self, target: &ResourceRef, to: Option<i64>) -> Result<String, String> {
        let command = Command::WorkloadRollback {
            target: target.clone(),
            to_revision: to,
        };
        match self.run(command, self.ctx()).await {
            Ok(Outcome::Completed(output)) => Ok(output.message.unwrap_or_default()),
            Ok(other) => panic!("expected completion, got {other:?}"),
            Err(DispatchError::Handler(error)) => Err(error.message().to_owned()),
            Err(other) => panic!("{other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deployment_with_three_revisions_rolls_back_to_the_previous_and_to_a_named_one() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin_client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let api = Api::<Deployment>::namespaced(admin_client.clone(), ns.name());
    api.create(&PostParams::default(), &deployment("web", images::PAUSE))
        .await
        .expect("create the deployment");
    crate::eventually("revision 1", String::new, || async {
        revision_of(&api, "web").await == "1"
    })
    .await;
    set_image(&api, "web", images::PAUSE_PREVIOUS, Some("second image")).await;
    crate::eventually("revision 2", String::new, || async {
        revision_of(&api, "web").await == "2"
    })
    .await;
    set_image(&api, "web", images::E2E_BUSYBOX, Some("third image")).await;
    crate::eventually("revision 3", String::new, || async {
        revision_of(&api, "web").await == "3"
    })
    .await;

    let rig = Rig::open(&kind).await;
    let web = rig.target("Deployment", ns.name(), "web");
    let service = RolloutService::new(rig.manager.clone());

    // --- the history: newest first, change causes, the current one marked ----------------------
    let history = service.history(&web).await.expect("history");
    let rows: Vec<_> = history
        .revisions
        .iter()
        .map(|r| {
            (
                r.revision,
                r.current,
                r.change_cause.clone(),
                r.images.clone(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                3,
                true,
                Some("third image".to_owned()),
                vec![images::E2E_BUSYBOX.to_owned()]
            ),
            (
                2,
                false,
                Some("second image".to_owned()),
                vec![images::PAUSE_PREVIOUS.to_owned()]
            ),
            (
                1,
                false,
                Some("first image".to_owned()),
                vec![images::PAUSE.to_owned()]
            ),
        ]
    );
    assert_eq!(history.preselected, Some(2));

    // --- a dry-run dispatch changes nothing -----------------------------------------------------
    let dry = rig
        .run(
            Command::WorkloadRollback {
                target: web.clone(),
                to_revision: None,
            },
            rig.ctx().with_dry_run(true),
        )
        .await
        .expect("dry run");
    let Outcome::Completed(output) = dry else {
        panic!("expected completion");
    };
    assert!(
        output
            .message
            .unwrap()
            .starts_with("Dry run: would roll back")
    );
    assert_eq!(image_of(&api, "web").await, images::E2E_BUSYBOX);
    assert_eq!(revision_of(&api, "web").await, "3");

    // --- undo to the previous revision ----------------------------------------------------------
    let message = rig.rollback(&web, None).await.expect("roll back");
    assert_eq!(
        message,
        format!(
            "Rolled back Deployment {}/web from revision 3 to revision 2",
            ns.name()
        )
    );
    assert_eq!(image_of(&api, "web").await, images::PAUSE_PREVIOUS);
    // The controller reuses revision 2's ReplicaSet and records it as the newest revision.
    crate::eventually("revision 4", String::new, || async {
        revision_of(&api, "web").await == "4"
    })
    .await;
    let live = api.get("web").await.unwrap();
    let labels = live
        .spec
        .unwrap()
        .template
        .metadata
        .unwrap()
        .labels
        .unwrap();
    assert!(!labels.contains_key("pod-template-hash"), "{labels:?}");
    assert_eq!(
        live.metadata
            .annotations
            .unwrap()
            .get(CAUSE)
            .map(String::as_str),
        Some("second image"),
        "the change cause comes back with the template"
    );

    // --- undo to a named revision ---------------------------------------------------------------
    let message = rig.rollback(&web, Some(1)).await.expect("roll back to 1");
    assert!(
        message.ends_with("from revision 4 to revision 1"),
        "{message}"
    );
    assert_eq!(image_of(&api, "web").await, images::PAUSE);
    crate::eventually("revision 5", String::new, || async {
        revision_of(&api, "web").await == "5"
    })
    .await;

    // --- the template already running ----------------------------------------------------------
    let message = rig.rollback(&web, Some(5)).await.expect("already there");
    assert!(message.ends_with("is already at revision 5"), "{message}");
    assert_eq!(revision_of(&api, "web").await, "5", "nothing was sent");

    // --- a revision that does not exist, and a paused Deployment --------------------------------
    let error = rig
        .rollback(&web, Some(99))
        .await
        .expect_err("no such revision");
    assert!(error.contains("unable to find revision 99"), "{error}");
    api.patch(
        "web",
        &PatchParams::default(),
        &Patch::Merge(json!({"spec": {"paused": true}})),
    )
    .await
    .expect("pause");
    let error = rig.rollback(&web, None).await.expect_err("paused");
    assert!(
        error.starts_with("you cannot rollback a paused deployment; resume it first"),
        "{error}"
    );
    assert_eq!(image_of(&api, "web").await, images::PAUSE);

    // --- the audit trail ------------------------------------------------------------------------
    let audit = rig.state.audit_log();
    // The commits' records: the dry-run dispatch and the guard's dry runs before it asks
    // (E19-S07) are dry-run records.
    let ok: Vec<_> = audit
        .iter()
        .filter(|r| r.outcome == AuditOutcome::Succeeded && !r.dry_run)
        .map(|r| r.detail.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(
        ok,
        [
            "to_revision=previous from=3 to=2",
            "to_revision=1 from=4 to=1",
            "to_revision=5 from=5 to=5"
        ],
        "{audit:?}"
    );
    assert!(audit.iter().all(|r| r.initiator == Initiator::Ui));
    assert_eq!(
        audit
            .iter()
            .filter(|r| r.outcome == AuditOutcome::Failed)
            .count(),
        2,
        "the missing revision and the paused Deployment"
    );

    // --- read-only: refused, untouched ----------------------------------------------------------
    rig.manager
        .set_read_only(&rig.cluster, true)
        .expect("read-only on");
    let error = rig
        .bus
        .dispatch(
            Command::WorkloadRollback {
                target: web.clone(),
                to_revision: Some(1),
            },
            rig.ctx(),
        )
        .await
        .expect_err("read-only refuses");
    assert!(matches!(error, DispatchError::ReadOnly { .. }), "{error:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_statefulset_rolls_back_through_its_controller_revisions() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin_client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let api = Api::<StatefulSet>::namespaced(admin_client.clone(), ns.name());
    api.create(&PostParams::default(), &statefulset("db", images::PAUSE))
        .await
        .expect("create the statefulset");

    let rig = Rig::open(&kind).await;
    let db = rig.target("StatefulSet", ns.name(), "db");
    let service = RolloutService::new(rig.manager.clone());
    crate::eventually("one controller revision", String::new, || async {
        service
            .history(&db)
            .await
            .is_ok_and(|h| h.revisions.len() == 1)
    })
    .await;
    set_image(&api, "db", images::PAUSE_PREVIOUS, None).await;
    crate::eventually("two controller revisions", String::new, || async {
        service
            .history(&db)
            .await
            .is_ok_and(|h| h.revisions.len() == 2)
    })
    .await;

    let history = service.history(&db).await.expect("history");
    assert_eq!(history.revisions[0].images, [images::PAUSE_PREVIOUS]);
    assert_eq!(history.revisions[1].images, [images::PAUSE]);

    let message = rig.rollback(&db, None).await.expect("roll back");
    assert!(
        message.contains("StatefulSet") && message.contains("to revision 1"),
        "{message}"
    );
    let live = api.get("db").await.unwrap();
    let image = live.spec.unwrap().template.spec.unwrap().containers[0]
        .image
        .clone();
    assert_eq!(image.as_deref(), Some(images::PAUSE));

    // The StatefulSet's own history now has the restored revision as the newest.
    crate::eventually("the restored revision is current", String::new, || async {
        service
            .history(&db)
            .await
            .is_ok_and(|h| h.current().is_some_and(|c| c.images == [images::PAUSE]))
    })
    .await;
}
