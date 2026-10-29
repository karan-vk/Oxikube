//! A multi-object apply (E20-S03) against the real adapter: `ApplyService::plan`, `dry_run_plan`
//! and `apply_plan` through the `CommandBus` and `MutationGuard` on a kind cluster, no UI.
//!
//! * a plan given in the wrong order (a custom resource, its definition, a ConfigMap and a
//!   Deployment in a namespace the plan also makes) is ordered by kind; its dry run defers what
//!   needs the new namespace or definition;
//! * the apply creates every object under the field manager `oxikube`, waits for the definition to
//!   be `Established` before the custom resource, and audits each object under one batch id;
//! * applying the same plan again is a server-side apply no-op: every object is unchanged (this
//!   is what kind's API server answers, the reason the detection reads the version);
//! * a changed value is configured; a read-only session refuses the whole plan.
//!
//! The namespace and the definition have unique names and are deleted when the test ends, also
//! when it panics; the Deployment has no replicas, so nothing is scheduled.

use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_app::apply::{
    ApplyError, ApplyOptions, ApplyService, ObjectStatus, PlanReport, PlanTarget, register_commands,
};
use oxikube_app::command_bus::CommandRegistry;
use oxikube_app::guard::{ConfirmationProvider, ConfirmationRequest, Decision};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{CommandBus, MutationGuard};
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::manifests::{ManifestDoc, SourceId};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::integration::random_namespace_name;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use serde_json::{Value, json};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// Deletes the plan's namespace and definition (best effort, no wait) when dropped.
struct Cleanup {
    context: String,
    namespace: String,
    crd: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for (kind, name) in [("namespace", &self.namespace), ("crd", &self.crd)] {
            let _ = Command::new("kubectl")
                .args(["--context", &self.context, "delete", kind, name])
                .args(["--ignore-not-found", "--wait=false"])
                .output();
        }
    }
}

/// Confirms whatever it is asked.
pub(crate) struct Yes;

#[async_trait]
impl ConfirmationProvider for Yes {
    async fn confirm(&self, request: &ConfirmationRequest) -> Decision {
        match &request.expected_name {
            Some(name) => Decision::typed(name.clone()),
            None => Decision::confirm(),
        }
    }
}

pub(crate) struct Rig {
    pub(crate) service: ApplyService,
    pub(crate) manager: ClusterSessionManager,
    pub(crate) state: Arc<FakeStatePort>,
    pub(crate) target: PlanTarget,
}

impl Rig {
    pub(crate) async fn new(kind: &Kind) -> Self {
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
            .install("oxikube_app::apply", move |r| {
                register_commands(r, handler_sessions)
            })
            .expect("register the apply commands");
        let bus = CommandBus::new(
            registry,
            MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
        );
        manager.open(&entry, SessionOptions::default());
        manager.connect(&entry.cluster).await.expect("connect");
        Self {
            service: ApplyService::new(bus, manager.clone(), "tester"),
            manager,
            state,
            target: PlanTarget {
                cluster: entry.cluster,
                namespace: None,
            },
        }
    }

    async fn dry_run(&self, objects: &[Value]) -> Result<PlanReport, ApplyError> {
        let plan = self.service.plan(docs(objects), self.target.clone()).await;
        self.service
            .dry_run_plan(&plan, &ApplyOptions::default())
            .into_report()
            .await
    }

    async fn apply(&self, objects: &[Value]) -> Result<PlanReport, ApplyError> {
        let plan = self.service.plan(docs(objects), self.target.clone()).await;
        self.service
            .apply_plan(&plan, &ApplyOptions::default(), Arc::new(Yes))
            .into_report()
            .await
    }
}

fn docs(objects: &[Value]) -> Vec<ManifestDoc> {
    objects
        .iter()
        .enumerate()
        .map(|(index, json)| ManifestDoc {
            source: SourceId::new("kind/apply-plan.yaml"),
            line: index + 1,
            json: json.clone(),
        })
        .collect()
}

fn words(report: &PlanReport) -> Vec<(&str, &'static str)> {
    report
        .objects
        .iter()
        .map(|o| (o.label.as_str(), o.status.word()))
        .collect()
}

/// The plan, given out of order: the custom resource first, the namespace last.
fn objects(namespace: &str, group: &str, mode: &str) -> Vec<Value> {
    vec![
        json!({"apiVersion": format!("{group}/v1"), "kind": "Gizmo",
            "metadata": {"name": "g1", "namespace": namespace}, "spec": {"size": 3}}),
        json!({"apiVersion": "apiextensions.k8s.io/v1", "kind": "CustomResourceDefinition",
            "metadata": {"name": format!("gizmos.{group}")},
            "spec": {"group": group, "scope": "Namespaced",
                "names": {"kind": "Gizmo", "plural": "gizmos", "singular": "gizmo"},
                "versions": [{"name": "v1", "served": true, "storage": true,
                    "schema": {"openAPIV3Schema": {"type": "object",
                        "x-kubernetes-preserve-unknown-fields": true}}}]}}),
        json!({"apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "settings", "namespace": namespace}, "data": {"mode": mode}}),
        json!({"apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": "web", "namespace": namespace},
            "spec": {"replicas": 0, "selector": {"matchLabels": {"app": "web"}},
                "template": {"metadata": {"labels": {"app": "web"}},
                    "spec": {"containers": [{"name": "web",
                        "image": oxikube_testkit::images::PAUSE}]}}}}),
        json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": namespace}}),
    ]
}

#[tokio::test]
async fn a_plan_is_ordered_dry_run_applied_and_idempotent() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let namespace = random_namespace_name();
    let group = format!("{namespace}.oxikube.test");
    let _cleanup = Cleanup {
        context: kind.context.to_string(),
        namespace: namespace.clone(),
        crd: format!("gizmos.{group}"),
    };
    let rig = Rig::new(&kind).await;

    let dry = rig
        .dry_run(&objects(&namespace, &group, "blue"))
        .await
        .expect("dry run");
    let ns_label = format!("Namespace {namespace}");
    let crd_label = format!("CustomResourceDefinition gizmos.{group}");
    let cm_label = format!("ConfigMap {namespace}/settings");
    let deploy_label = format!("Deployment {namespace}/web");
    let gizmo_label = format!("Gizmo {namespace}/g1");
    assert_eq!(
        words(&dry),
        [
            (ns_label.as_str(), "created"),
            (cm_label.as_str(), "deferred"),
            (crd_label.as_str(), "created"),
            (deploy_label.as_str(), "deferred"),
            (gizmo_label.as_str(), "deferred"),
        ],
        "{dry:#?}"
    );

    let applied = rig
        .apply(&objects(&namespace, &group, "blue"))
        .await
        .expect("applied");
    assert!(
        applied
            .objects
            .iter()
            .all(|o| o.status == ObjectStatus::Created),
        "{applied:#?}"
    );
    let records = rig.state.audit_log();
    let batch: Vec<_> = records
        .iter()
        .filter(|r| r.cmd.as_ref() == "resource::Apply" && !r.dry_run)
        .collect();
    assert_eq!(batch.len(), 5, "one record per object");
    assert!(batch.iter().all(|r| r.outcome == AuditOutcome::Succeeded
        && r.batch_id.is_some()
        && r.batch_id == batch[0].batch_id));

    // The same plan again: a server-side apply of what is already there writes nothing.
    let again = rig
        .apply(&objects(&namespace, &group, "blue"))
        .await
        .expect("applied again");
    assert!(
        again
            .objects
            .iter()
            .all(|o| o.status == ObjectStatus::Unchanged),
        "{again:#?}"
    );

    // One value changed: that ConfigMap is configured, the rest unchanged.
    let changed = rig
        .apply(&objects(&namespace, &group, "green"))
        .await
        .expect("applied a change");
    let configured: Vec<_> = changed
        .objects
        .iter()
        .filter(|o| o.status == ObjectStatus::Configured)
        .map(|o| o.label.as_str())
        .collect();
    assert_eq!(configured, [cm_label.as_str()], "{changed:#?}");

    // Read-only: the whole plan is refused before any request.
    rig.manager
        .set_read_only(&rig.target.cluster, true)
        .expect("read-only");
    let refused = rig.apply(&objects(&namespace, &group, "red")).await;
    assert!(
        matches!(refused, Err(ApplyError::ReadOnly { .. })),
        "{refused:?}"
    );
}
