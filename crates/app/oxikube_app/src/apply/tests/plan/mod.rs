//! Many objects as one plan (E20-S03), over the real guard, the real handlers and the fakes:
//! order, the dry run first, the guard (one confirmation, read-only, audit), what each object did,
//! the definitions a custom resource waits for, stop on error and cancel.

mod crd;
mod fixture;
mod flow;
mod guard;
mod outcomes;
mod stop;

use std::sync::Arc;

use async_trait::async_trait;
use futures::FutureExt as _;
use oxikube_domain::manifests::{ManifestDoc, SourceId};
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::Applying;
use crate::apply::{
    ApplyError, ApplyOptions, ClusterPlan, ObjectResult, ObjectStatus, PlanReport, PlanTarget,
};
use crate::guard::{ConfirmationProvider, ConfirmationRequest, Decision};
use crate::testing::id;

pub(super) fn namespace(name: &str) -> Value {
    json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": name}})
}

pub(super) fn configmap(name: &str, namespace: Option<&str>) -> Value {
    let mut object = json!({"apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": name}, "data": {"mode": "blue"}});
    if let Some(namespace) = namespace {
        object["metadata"]["namespace"] = json!(namespace);
    }
    object
}

pub(super) fn deployment(name: &str) -> Value {
    json!({"apiVersion": "apps/v1", "kind": "Deployment", "metadata": {"name": name},
        "spec": {"replicas": 1}})
}

pub(super) fn crd() -> Value {
    json!({"apiVersion": "apiextensions.k8s.io/v1", "kind": "CustomResourceDefinition",
        "metadata": {"name": "widgets.example.com"},
        "spec": {"group": "example.com", "scope": "Namespaced",
            "names": {"kind": "Widget", "plural": "widgets"}}})
}

pub(super) fn widget(name: &str) -> Value {
    json!({"apiVersion": "example.com/v1", "kind": "Widget", "metadata": {"name": name}})
}

/// Answers every confirmation with one decision and keeps what it was asked.
pub struct Answering {
    decision: Decision,
    pub asked: Mutex<Vec<ConfirmationRequest>>,
}

impl Answering {
    pub fn confirm() -> Arc<Self> {
        Self::with(Decision::confirm())
    }

    pub fn decline() -> Arc<Self> {
        Self::with(Decision::Decline)
    }

    fn with(decision: Decision) -> Arc<Self> {
        Arc::new(Self {
            decision,
            asked: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl ConfirmationProvider for Answering {
    async fn confirm(&self, request: &ConfirmationRequest) -> Decision {
        self.asked.lock().push(request.clone());
        match (&self.decision, &request.expected_name) {
            (Decision::Confirm { .. }, Some(name)) => Decision::typed(name.clone()),
            (decision, _) => decision.clone(),
        }
    }
}

impl Applying {
    /// The plan of `objects` for cluster `a`, default namespace.
    pub fn plan(&self, objects: Vec<Value>) -> ClusterPlan {
        self.plan_in(objects, None)
    }

    pub fn plan_in(&self, objects: Vec<Value>, namespace: Option<&str>) -> ClusterPlan {
        let docs = objects
            .into_iter()
            .enumerate()
            .map(|(index, json)| ManifestDoc {
                source: SourceId::new("dir/app.yaml"),
                line: index * 10 + 1,
                json,
            })
            .collect();
        let target = PlanTarget {
            cluster: id("a"),
            namespace: namespace.map(str::to_owned),
        };
        self.service
            .plan(docs, target)
            .now_or_never()
            .expect("the fakes answer at once")
    }

    pub fn dry_run_plan(
        &self,
        plan: &ClusterPlan,
        options: &ApplyOptions,
    ) -> Result<PlanReport, ApplyError> {
        self.service
            .dry_run_plan(plan, options)
            .into_report()
            .now_or_never()
            .expect("the fakes answer at once")
    }

    pub fn apply_plan(
        &self,
        plan: &ClusterPlan,
        options: &ApplyOptions,
        confirm: Arc<Answering>,
    ) -> Result<PlanReport, ApplyError> {
        self.service
            .apply_plan(plan, options, confirm)
            .into_report()
            .now_or_never()
            .expect("the fakes answer at once")
    }

    /// The names of the objects written for real, in order.
    pub fn applied_names(&self) -> Vec<String> {
        self.h
            .writes("a")
            .iter()
            .filter(|call| !call.is_dry_run())
            .map(|call| {
                super::body(call)["metadata"]["name"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }
}

/// The status words of `report`, in plan order.
pub(super) fn words(report: &PlanReport) -> Vec<&'static str> {
    report.objects.iter().map(|o| o.status.word()).collect()
}

/// The labels of `report`, in plan order.
pub(super) fn labels(report: &PlanReport) -> Vec<&str> {
    report.objects.iter().map(|o| o.label.as_str()).collect()
}

/// The reason of a skipped or failed object.
pub(super) fn reason(result: &ObjectResult) -> &str {
    match &result.status {
        ObjectStatus::Skipped { reason } | ObjectStatus::Failed { reason, .. } => reason,
        other => panic!("not a failure: {other:?}"),
    }
}
