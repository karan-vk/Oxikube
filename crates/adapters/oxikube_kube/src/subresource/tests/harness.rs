//! Fixtures: a fake API server whose discovery serves `Pod`, `Deployment`, `Node`, the `Widget`
//! CRD and a `Secret`.

use serde_json::{Value, json};

use crate::discovery::{DiscoveryConfig, KubeDiscovery};
use crate::fake_api::{FakeApi, Recorded};
use crate::resources::KubeResources;
use oxikube_domain::ids::Gvk;

pub(super) const DEPLOY_SCALE: &str = "/apis/apps/v1/namespaces/default/deployments/web/scale";
pub(super) const DEPLOY: &str = "/apis/apps/v1/namespaces/default/deployments/web";
pub(super) const WIDGET: &str = "/apis/test.oxikube.dev/v1/namespaces/default/widgets/w";
pub(super) const WIDGET_SCALE: &str =
    "/apis/test.oxikube.dev/v1/namespaces/default/widgets/w/scale";
pub(super) const WIDGET_STATUS: &str =
    "/apis/test.oxikube.dev/v1/namespaces/default/widgets/w/status";
pub(super) const POD_EVICTION: &str = "/api/v1/namespaces/default/pods/p/eviction";
pub(super) const POD_EPHEMERAL: &str = "/api/v1/namespaces/default/pods/p/ephemeralcontainers";
pub(super) const POD_RESIZE: &str = "/api/v1/namespaces/default/pods/p/resize";
pub(super) const NODE_STATUS: &str = "/api/v1/nodes/n1/status";

pub(super) fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

pub(super) fn node_gvk() -> Gvk {
    Gvk::new("", "v1", "Node")
}

pub(super) fn deployment_gvk() -> Gvk {
    Gvk::new("apps", "v1", "Deployment")
}

pub(super) fn widget_gvk() -> Gvk {
    Gvk::new("test.oxikube.dev", "v1", "Widget")
}

fn resource(name: &str, kind: &str, namespaced: bool, verbs: &[&str]) -> Value {
    json!({"name": name, "singularName": "", "namespaced": namespaced, "kind": kind, "verbs": verbs})
}

fn group(name: &str, version: &str) -> Value {
    let gv = json!({"groupVersion": format!("{name}/{version}"), "version": version});
    json!({"name": name, "versions": [gv], "preferredVersion": gv})
}

/// A server with discovery scripted and nothing else.
pub(super) fn server() -> FakeApi {
    let api = FakeApi::new();
    let all = [
        "get", "list", "watch", "create", "update", "patch", "delete",
    ];
    api.reply(
        "/api",
        200,
        json!({"kind": "APIVersions", "versions": ["v1"]}),
    );
    api.reply(
        "/apis",
        200,
        json!({"kind": "APIGroupList", "groups": [group("apps", "v1"), group("test.oxikube.dev", "v1")]}),
    );
    api.reply(
        "/api/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "v1", "resources": [
            resource("pods", "Pod", true, &all),
            resource("nodes", "Node", false, &all),
            // Read-only: a subresource write is still checked against `patch`.
            resource("secrets", "Secret", true, &["get", "list"]),
        ]}),
    );
    api.reply(
        "/apis/apps/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "apps/v1", "resources": [
            resource("deployments", "Deployment", true, &all),
        ]}),
    );
    api.reply(
        "/apis/test.oxikube.dev/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "test.oxikube.dev/v1", "resources": [
            resource("widgets", "Widget", true, &all),
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

/// An `autoscaling/v1` `Scale` as the server returns it.
pub(super) fn scale_body(spec: i64, status: i64, rv: &str) -> Value {
    json!({
        "apiVersion": "autoscaling/v1", "kind": "Scale",
        "metadata": {"name": "web", "namespace": "default", "resourceVersion": rv},
        "spec": {"replicas": spec},
        "status": {"replicas": status, "selector": "app=web"},
    })
}

/// The only request to `path`.
pub(super) fn only(api: &FakeApi, path: &str) -> Recorded {
    let mut hits: Vec<_> = api
        .requests()
        .into_iter()
        .filter(|r| r.path == path)
        .collect();
    assert_eq!(hits.len(), 1, "expected one request to {path}");
    hits.remove(0)
}

/// The decoded, sorted query pairs of `request`.
pub(super) fn query(request: &Recorded) -> Vec<String> {
    let mut pairs: Vec<String> = url::form_urlencoded::parse(request.query.as_bytes())
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    pairs.sort();
    pairs
}

/// The requests that are not discovery.
pub(super) fn calls(api: &FakeApi) -> Vec<Recorded> {
    const DISCOVERY: [&str; 5] = [
        "/api",
        "/apis",
        "/api/v1",
        "/apis/apps/v1",
        "/apis/test.oxikube.dev/v1",
    ];
    api.requests()
        .into_iter()
        .filter(|r| !DISCOVERY.contains(&r.path.as_str()))
        .collect()
}
