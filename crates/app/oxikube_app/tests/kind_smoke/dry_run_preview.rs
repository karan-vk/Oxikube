//! The guard's dry-run preview (E19-S07) against the real adapter: a create, update, patch or
//! apply is dry-run by the guard (`dryRun=All`) before it is confirmed, and the question carries
//! the live object next to the server's result.
//!
//! * a Deployment's replicas changed through `resource::Apply` (server-side apply) and its image
//!   through `workload::SetImage` (a strategic merge patch): the preview shows the change and the
//!   cluster is unchanged afterwards; a declined question changes nothing either;
//! * a field another manager owns: the server refuses the preview with the conflict and its
//!   manager, and nothing is asked;
//! * a custom resource of a CRD without a schema (`x-kubernetes-preserve-unknown-fields`) is
//!   previewed as a create and then created.
//!
//! Every object lives in the test's own `oxi-test-<rand>` namespace; the CRD has a unique name and
//! is deleted when the test ends.

use std::process::Command as Process;
use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::Api;
use kube::api::{Patch as KubePatch, PatchParams, PostParams};
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{CommandBus, ConfirmationRequest, DryRunPreview, DryRunResult, MutationGuard};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, ImageChange};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::{ConflictReason, ErrorKind};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::images::PAUSE;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use serde_json::{Value, json};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// A bus with the real apply and workload handlers over a session on the kind cluster.
struct Rig {
    bus: CommandBus,
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
        let sessions = manager.clone();
        registry
            .install("oxikube_app::apply", move |r| {
                oxikube_app::apply::register_commands(r, sessions)
            })
            .expect("register resource::Apply");
        let sessions = manager.clone();
        registry
            .install("oxikube_app::workloads", move |r| {
                oxikube_app::workloads::register_commands(r, sessions)
            })
            .expect("register the workload commands");
        let bus = CommandBus::new(
            registry,
            MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
        );
        manager.open(&entry, SessionOptions::default());
        manager.connect(&entry.cluster).await.expect("connect");
        Self {
            bus,
            manager,
            state,
            cluster: entry.cluster,
        }
    }

    fn ctx(&self, initiator: Initiator) -> DispatchContext {
        DispatchContext::new(initiator, "tester").with_cluster(self.cluster.clone())
    }

    fn apply(&self, manifest: &Value) -> Command {
        Command::ResourceApply {
            cluster: self.cluster.clone(),
            namespace: None,
            manifest: manifest.to_string(),
            field_manager: None,
            force: false,
            resource_version: None,
            reviewed_absent: false,
        }
    }

    /// The first dispatch of `command`: the guard's question, with its dry run.
    async fn ask(&self, command: &Command) -> (ConfirmationRequest, DryRunResult) {
        let outcome = self
            .bus
            .dispatch(command.clone(), self.ctx(Initiator::Agent))
            .await
            .expect("asks");
        let Outcome::NeedsConfirmation(request) = outcome else {
            panic!("a previewed command asks first: {outcome:?}");
        };
        let Some(DryRunPreview::Rehearsed(preview)) = request.preview.clone() else {
            panic!("the question carries the guard's dry run: {request:?}");
        };
        (request, preview)
    }

    async fn confirm(&self, command: Command, request: &ConfirmationRequest) -> Outcome {
        self.bus
            .dispatch(
                command,
                self.ctx(Initiator::Agent)
                    .with_confirmation(Confirmation::simple(request.token)),
            )
            .await
            .expect("confirmed")
    }
}

fn deployment(namespace: &str, replicas: i32) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {"name": "web", "namespace": namespace},
        "spec": {
            "replicas": replicas,
            "selector": {"matchLabels": {"app": "web"}},
            "template": {
                "metadata": {"labels": {"app": "web"}},
                "spec": {
                    "terminationGracePeriodSeconds": 0,
                    // No node has this label: nothing is scheduled, the spec is all that matters.
                    "nodeSelector": {"oxikube.dev/none": "none"},
                    "containers": [{"name": "main", "image": PAUSE}]
                }
            }
        }
    })
}

async fn live(api: &Api<Deployment>) -> Value {
    serde_json::to_value(api.get("web").await.expect("the Deployment")).expect("json")
}

#[tokio::test]
async fn a_patch_and_an_apply_are_previewed_and_leave_the_cluster_unchanged() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let admin = kind.admin_client().await;
    let deployments = Api::<Deployment>::namespaced(admin, ns.name());
    // Created by server-side apply under the app's own field manager, so the apply below owns
    // the fields it changes and the server does not report a conflict.
    deployments
        .patch(
            "web",
            &PatchParams::apply("oxikube"),
            &KubePatch::Apply(deployment(ns.name(), 1)),
        )
        .await
        .expect("create the Deployment");
    let rig = Rig::new(&kind).await;
    let before = live(&deployments).await;

    // Server-side apply of replicas 3: previewed against the live object.
    let apply = rig.apply(&deployment(ns.name(), 3));
    let (request, preview) = rig.ask(&apply).await;
    let [change] = preview.changes.as_slice() else {
        panic!("one object: {preview:?}");
    };
    assert_eq!(change.change, oxikube_app::apply::Change::Update);
    assert_eq!(change.live.as_deref().unwrap()["spec"]["replicas"], 1);
    assert_eq!(change.result["spec"]["replicas"], 3);
    // Declined: nothing changed, the dry run and the refusal are audited.
    rig.bus.decline(request.token).await.expect("declined");

    // A strategic merge patch of the image: previewed the same way.
    let target = ResourceRef::namespaced(
        rig.cluster.clone(),
        Gvk::new("apps", "v1", "Deployment"),
        ns.name(),
        "web",
    );
    let set_image = Command::WorkloadSetImage {
        target,
        images: vec![ImageChange::new("main", "registry.k8s.io/pause:dry-run")],
    };
    let (_, preview) = rig.ask(&set_image).await;
    let result = &preview.changes[0].result;
    assert_eq!(
        result["spec"]["template"]["spec"]["containers"][0]["image"],
        "registry.k8s.io/pause:dry-run"
    );

    // The controller still writes the status; the object itself is as it was.
    let after = live(&deployments).await;
    assert_eq!(
        after["metadata"]["generation"], before["metadata"]["generation"],
        "the dry runs changed nothing"
    );
    assert_eq!(after["spec"], before["spec"]);
    assert_eq!(after["spec"]["replicas"], 1);
    let audit = rig.state.audit_log();
    let summary: Vec<(bool, AuditOutcome)> = audit.iter().map(|r| (r.dry_run, r.outcome)).collect();
    assert_eq!(
        summary,
        [
            (true, AuditOutcome::Succeeded),
            (false, AuditOutcome::Cancelled),
            (true, AuditOutcome::Succeeded),
        ]
    );
    rig.manager.disconnect(&rig.cluster).expect("disconnect");
}

#[tokio::test]
async fn a_field_another_manager_owns_is_refused_at_the_preview_with_its_manager() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let admin = kind.admin_client().await;
    let deployments = Api::<Deployment>::namespaced(admin, ns.name());
    // `helm-like` owns `spec.replicas` through server-side apply.
    deployments
        .patch(
            "web",
            &PatchParams::apply("helm-like"),
            &KubePatch::Apply(deployment(ns.name(), 2)),
        )
        .await
        .expect("apply as another manager");
    let rig = Rig::new(&kind).await;

    let error = rig
        .bus
        .dispatch(
            rig.apply(&deployment(ns.name(), 5)),
            rig.ctx(Initiator::Agent),
        )
        .await
        .expect_err("the server refuses the dry run");
    let DispatchError::Handler(error) = &error else {
        panic!("the server's refusal: {error:?}");
    };
    assert_eq!(error.kind(), ErrorKind::Conflict);
    let details = error.conflict_details().expect("the conflict's details");
    assert_eq!(details.reason, ConflictReason::FieldOwnership);
    assert!(
        details
            .causes
            .iter()
            .any(|cause| cause.manager.as_deref() == Some("helm-like")),
        "{details:?}"
    );
    assert_eq!(rig.bus.guard().pending_confirmations(), 0, "nothing asked");
    assert_eq!(live(&deployments).await["spec"]["replicas"], 2);
    rig.manager.disconnect(&rig.cluster).expect("disconnect");
}

/// Deletes the CRD (best effort, no wait) when dropped, also when the test panics.
struct DeleteCrd {
    context: String,
    name: String,
}

impl Drop for DeleteCrd {
    fn drop(&mut self) {
        let _ = Process::new("kubectl")
            .args(["--context", &self.context, "delete", "crd", &self.name])
            .args(["--ignore-not-found", "--wait=false"])
            .output();
    }
}

#[tokio::test]
async fn a_custom_resource_without_a_schema_is_previewed_and_created() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let group = format!("preview-{:x}.test.oxikube.dev", std::process::id() ^ nanos);
    let name = format!("gadgets.{group}");
    let _cleanup = DeleteCrd {
        context: kind.context.to_string(),
        name: name.clone(),
    };
    let crd: CustomResourceDefinition = serde_json::from_value(json!({
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": {"name": name},
        "spec": {
            "group": group,
            "scope": "Namespaced",
            "names": {"plural": "gadgets", "singular": "gadget", "kind": "Gadget",
                      "listKind": "GadgetList"},
            "versions": [{
                "name": "v1", "served": true, "storage": true,
                "schema": {"openAPIV3Schema": {"type": "object",
                           "x-kubernetes-preserve-unknown-fields": true}},
            }],
        },
    }))
    .expect("crd json");
    let admin = kind.admin_client().await;
    Api::<CustomResourceDefinition>::all(admin)
        .create(&PostParams::default(), &crd)
        .await
        .expect("create the CRD");
    let rig = Rig::new(&kind).await;
    let manifest = json!({
        "apiVersion": format!("{group}/v1"),
        "kind": "Gadget",
        "metadata": {"name": "sprocket", "namespace": ns.name()},
        "spec": {"anything": {"goes": [1, 2, 3]}}
    });
    let apply = rig.apply(&manifest);

    // The CRD's kind reaches discovery shortly after it is established.
    let deadline = std::time::Instant::now() + crate::DEADLINE;
    let (request, preview) = loop {
        match rig
            .bus
            .dispatch(apply.clone(), rig.ctx(Initiator::Agent))
            .await
        {
            Ok(Outcome::NeedsConfirmation(request))
                if let Some(DryRunPreview::Rehearsed(preview)) = request.preview.clone() =>
            {
                break (request, preview);
            }
            // Until discovery serves the kind, the adapter cannot even address it (an
            // `Unsupported` error, so no preview): decline and ask again.
            Ok(Outcome::NeedsConfirmation(request)) if std::time::Instant::now() < deadline => {
                rig.bus.decline(request.token).await.expect("declined");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            other if std::time::Instant::now() < deadline => {
                tracing::debug!(?other, "the CRD is not served yet");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            other => panic!("the custom resource was never previewed: {other:?}"),
        }
    };
    let [change] = preview.changes.as_slice() else {
        panic!("one object: {preview:?}");
    };
    assert_eq!(change.change, oxikube_app::apply::Change::Create);
    assert_eq!(change.result["spec"]["anything"]["goes"], json!([1, 2, 3]));

    let outcome = rig.confirm(apply, &request).await;
    assert!(matches!(outcome, Outcome::Completed(_)), "{outcome:?}");
    let status = Process::new("kubectl")
        .args(["--context", kind.context.as_str(), "-n", ns.name()])
        .args(["get", &format!("gadgets.{group}"), "sprocket", "-o", "name"])
        .output()
        .expect("kubectl");
    assert!(status.status.success(), "{status:?}");
    rig.manager.disconnect(&rig.cluster).expect("disconnect");
}
