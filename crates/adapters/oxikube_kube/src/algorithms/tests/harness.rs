//! Fixtures: a fake API server whose discovery serves the kinds the algorithms touch, and
//! builders for the objects they read.

use oxikube_domain::Resource;
use serde_json::{Value, json};

use crate::discovery::{DiscoveryConfig, KubeDiscovery};
use crate::fake_api::{FakeApi, Recorded};
use crate::resources::KubeResources;

pub(super) const CRONJOB: &str = "/apis/batch/v1/namespaces/default/cronjobs/nightly";
pub(super) const JOBS: &str = "/apis/batch/v1/namespaces/default/jobs";
pub(super) const DEPLOY: &str = "/apis/apps/v1/namespaces/default/deployments/web";
pub(super) const REPLICASETS: &str = "/apis/apps/v1/namespaces/default/replicasets";
pub(super) const NODE: &str = "/api/v1/nodes/n1";
pub(super) const PODS: &str = "/api/v1/pods";

fn resource(name: &str, kind: &str, namespaced: bool) -> Value {
    json!({
        "name": name, "singularName": "", "namespaced": namespaced, "kind": kind,
        "verbs": ["get", "list", "watch", "create", "update", "patch", "delete"],
    })
}

fn group(name: &str, version: &str) -> Value {
    let gv = json!({"groupVersion": format!("{name}/{version}"), "version": version});
    json!({"name": name, "versions": [gv], "preferredVersion": gv})
}

/// A server with discovery scripted and nothing else.
pub(super) fn server() -> FakeApi {
    let api = FakeApi::new();
    api.reply(
        "/api",
        200,
        json!({"kind": "APIVersions", "versions": ["v1"]}),
    );
    api.reply(
        "/apis",
        200,
        json!({"kind": "APIGroupList", "groups": [group("apps", "v1"), group("batch", "v1")]}),
    );
    api.reply(
        "/api/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "v1", "resources": [
            resource("pods", "Pod", true), resource("nodes", "Node", false),
        ]}),
    );
    api.reply(
        "/apis/apps/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "apps/v1", "resources": [
            resource("deployments", "Deployment", true),
            resource("replicasets", "ReplicaSet", true),
        ]}),
    );
    api.reply(
        "/apis/batch/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "batch/v1", "resources": [
            resource("cronjobs", "CronJob", true), resource("jobs", "Job", true),
        ]}),
    );
    api
}

/// `KubeResources` over `api` with default settings.
pub(super) fn resources(api: &FakeApi) -> KubeResources {
    let discovery = KubeDiscovery::with_config(
        api.client(),
        DiscoveryConfig {
            aggregated: false,
            ..DiscoveryConfig::default()
        },
    );
    KubeResources::new(api.client(), discovery)
}

/// The requests that are not discovery.
pub(super) fn calls(api: &FakeApi) -> Vec<Recorded> {
    const DISCOVERY: [&str; 5] = [
        "/api",
        "/apis",
        "/api/v1",
        "/apis/apps/v1",
        "/apis/batch/v1",
    ];
    api.requests()
        .into_iter()
        .filter(|r| !DISCOVERY.contains(&r.path.as_str()))
        .collect()
}

/// The requests that change something.
pub(super) fn writes(api: &FakeApi) -> Vec<Recorded> {
    calls(api)
        .into_iter()
        .filter(|r| r.method != http::Method::GET)
        .collect()
}

/// The decoded, sorted query pairs of `request`.
pub(super) fn query(request: &Recorded) -> Vec<String> {
    let mut pairs: Vec<String> = url::form_urlencoded::parse(request.query.as_bytes())
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    pairs.sort();
    pairs
}

pub(super) fn object(value: Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

/// A CronJob with a job template carrying labels and annotations.
pub(super) fn cronjob_json() -> Value {
    json!({
        "apiVersion": "batch/v1", "kind": "CronJob",
        "metadata": {"name": "nightly", "namespace": "default", "uid": "cj-uid"},
        "spec": {
            "schedule": "0 3 * * *",
            "jobTemplate": {
                "metadata": {"labels": {"team": "data"}, "annotations": {"note": "x"}},
                "spec": {"backoffLimit": 2, "template": {"spec": {
                    "restartPolicy": "Never",
                    "containers": [{"name": "run", "image": "busybox:1"}],
                }}},
            },
        },
    })
}

/// A pod template; `hash` is the controller's `pod-template-hash` label when given.
pub(super) fn template(image: &str, hash: Option<&str>) -> Value {
    let mut labels = json!({"app": "web"});
    if let Some(hash) = hash {
        labels["pod-template-hash"] = json!(hash);
    }
    json!({
        "metadata": {"labels": labels},
        "spec": {"containers": [{"name": "web", "image": image}]},
    })
}

/// The Deployment `web` (uid `d-uid`) at `revision`, running `image`.
pub(super) fn deployment_json(revision: i64, image: &str) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {
            "name": "web", "namespace": "default", "uid": "d-uid",
            "annotations": {"deployment.kubernetes.io/revision": revision.to_string()},
        },
        "spec": {
            "selector": {"matchLabels": {"app": "web"}},
            "template": template(image, None),
        },
    })
}

/// A ReplicaSet of Deployment `owner_uid` at `revision`, holding `image`'s template.
pub(super) fn replicaset_json(
    name: &str,
    owner_uid: &str,
    revision: i64,
    image: &str,
    extra_annotations: Value,
) -> Value {
    let mut annotations = json!({"deployment.kubernetes.io/revision": revision.to_string()});
    for (key, value) in extra_annotations.as_object().into_iter().flatten() {
        annotations[key] = value.clone();
    }
    json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {
            "name": name, "namespace": "default", "uid": format!("{name}-uid"),
            "labels": {"app": "web"},
            "annotations": annotations,
            "creationTimestamp": "2026-01-01T00:00:00Z",
            "ownerReferences": [{
                "apiVersion": "apps/v1", "kind": "Deployment", "name": "web",
                "uid": owner_uid, "controller": true, "blockOwnerDeletion": true,
            }],
        },
        "spec": {"selector": {"matchLabels": {"app": "web"}}, "template": template(image, Some(name))},
    })
}

/// A pod on `node`, controlled by a ReplicaSet unless `owner_kind` says otherwise.
pub(super) fn pod_json(name: &str, node: &str, owner_kind: Option<&str>) -> Value {
    let mut pod = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": "default", "uid": format!("{name}-uid")},
        "spec": {"nodeName": node, "containers": [{"name": "c", "image": "pause"}]},
        "status": {"phase": "Running"},
    });
    if let Some(kind) = owner_kind {
        pod["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "apps/v1", "kind": kind, "name": "owner",
            "uid": "owner-uid", "controller": true,
        }]);
    }
    pod
}
