//! Shared by the drain tests: paths, object builders, the scripted servers and the helpers that
//! turn a drain stream into labelled steps.

use std::sync::Arc;

use futures::StreamExt as _;
use oxikube_domain::OxiResult;
use oxikube_ports::ResourcePort;
use serde_json::{Value, json};

use super::harness::*;
use crate::algorithms::{DrainOptions, DrainProgress, DrainSummary, drain};
use crate::fake_api::{FakeApi, status_body};

pub(super) fn pod_path(name: &str) -> String {
    format!("/api/v1/namespaces/default/pods/{name}")
}

pub(super) fn eviction_path(name: &str) -> String {
    format!("{}/eviction", pod_path(name))
}

pub(super) fn node_json(unschedulable: bool) -> Value {
    json!({
        "apiVersion": "v1", "kind": "Node",
        "metadata": {"name": "n1", "uid": "n1-uid"},
        "spec": {"unschedulable": unschedulable},
    })
}

pub(super) fn pod_list(pods: Vec<Value>) -> Value {
    json!({"apiVersion": "v1", "kind": "PodList", "metadata": {}, "items": pods})
}

pub(super) fn accepted() -> Value {
    json!({"kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Success", "code": 201})
}

pub(super) fn refused_by_budget() -> Value {
    json!({
        "kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Failure", "code": 429,
        "reason": "TooManyRequests",
        "message": "Cannot evict pod as it would violate the pod's disruption budget.",
        "details": {"causes": [{
            "reason": "DisruptionBudget",
            "message": "The disruption budget web needs 2 healthy pods and has 2 currently",
        }]},
    })
}

pub(super) fn gone() -> Value {
    status_body(404, "NotFound", "pods \"x\" not found")
}

/// A server where `names` are ReplicaSet pods on `n1` that evict at once and vanish at once.
pub(super) fn easy(names: &[&str], node: Value) -> FakeApi {
    let api = server();
    api.reply(NODE, 200, node);
    api.reply(NODE, 200, node_json(true));
    api.reply(
        PODS,
        200,
        pod_list(
            names
                .iter()
                .map(|n| pod_json(n, "n1", Some("ReplicaSet")))
                .collect(),
        ),
    );
    for name in names {
        api.reply(&eviction_path(name), 201, accepted());
        api.reply(&pod_path(name), 404, gone());
    }
    api
}

pub(super) async fn run(api: &FakeApi, options: DrainOptions) -> Vec<OxiResult<DrainProgress>> {
    let port: Arc<dyn ResourcePort> = Arc::new(resources(api));
    drain(port, "n1", options).collect().await
}

pub(super) fn progress(steps: Vec<OxiResult<DrainProgress>>) -> Vec<DrainProgress> {
    steps
        .into_iter()
        .map(|step| step.expect("no failure of the drain as a whole"))
        .collect()
}

/// A short label per event, `Evicting(a)`, for order assertions.
pub(super) fn label(step: &DrainProgress) -> String {
    match step {
        DrainProgress::Planned { .. } => "Planned".into(),
        DrainProgress::Cordoned { already } => format!("Cordoned({already})"),
        DrainProgress::Evicting { pod, attempt } => format!("Evicting({},{attempt})", pod.name),
        DrainProgress::Blocked { pod, attempt, .. } => format!("Blocked({},{attempt})", pod.name),
        DrainProgress::Evicted { pod } => format!("Evicted({})", pod.name),
        DrainProgress::Gone { pod } => format!("Gone({})", pod.name),
        DrainProgress::PodFailed { pod, .. } => format!("PodFailed({})", pod.name),
        DrainProgress::Finished(_) => "Finished".into(),
    }
}

pub(super) fn labels(steps: &[DrainProgress]) -> Vec<String> {
    steps.iter().map(label).collect()
}

pub(super) fn finished(steps: &[DrainProgress]) -> &DrainSummary {
    match steps.last() {
        Some(DrainProgress::Finished(summary)) => summary,
        other => panic!("the last step is not Finished: {other:?}"),
    }
}

pub(super) fn position(labels: &[String], wanted: &str) -> usize {
    labels
        .iter()
        .position(|l| l == wanted)
        .unwrap_or_else(|| panic!("{wanted} not in {labels:?}"))
}
