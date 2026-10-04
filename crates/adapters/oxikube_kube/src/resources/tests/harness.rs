//! Fixtures: a fake API server with discovery for `Pod`, `Namespace`, `Deployment`, the
//! `Widget` CRD and a create-only `Binding`.

use serde_json::{Value, json};

use crate::discovery::{DiscoveryConfig, KubeDiscovery};
use crate::fake_api::{FakeApi, status_body};
use crate::resources::{KubeResources, ResourcesConfig};
use oxikube_domain::ids::Gvk;

pub(super) const PODS: &str = "/api/v1/namespaces/default/pods";
pub(super) const ALL_PODS: &str = "/api/v1/pods";

pub(super) fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

pub(super) fn namespace_gvk() -> Gvk {
    Gvk::new("", "v1", "Namespace")
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
    let full = ["get", "list", "watch"];
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
            resource("pods", "Pod", true, &full),
            resource("namespaces", "Namespace", false, &full),
            resource("bindings", "Binding", true, &["create"]),
        ]}),
    );
    api.reply(
        "/apis/apps/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "apps/v1", "resources": [
            resource("deployments", "Deployment", true, &full),
        ]}),
    );
    api.reply(
        "/apis/test.oxikube.dev/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "test.oxikube.dev/v1", "resources": [
            resource("widgets", "Widget", true, &full),
        ]}),
    );
    api
}

/// `KubeResources` over `api` with default settings.
pub(super) fn resources(api: &FakeApi) -> KubeResources {
    resources_with(api, ResourcesConfig::default())
}

pub(super) fn resources_with(api: &FakeApi, config: ResourcesConfig) -> KubeResources {
    let discovery = KubeDiscovery::with_config(
        api.client(),
        DiscoveryConfig {
            aggregated: false,
            ..DiscoveryConfig::default()
        },
    );
    KubeResources::with_config(api.client(), discovery, config)
}

/// A pod as a list item (no `apiVersion`/`kind`, like a real list) with managed fields.
pub(super) fn pod_item(name: &str) -> Value {
    json!({
        "metadata": {
            "name": name, "namespace": "default", "uid": format!("uid-{name}"),
            "resourceVersion": "10", "labels": {"app": "web"},
            "creationTimestamp": "2026-01-02T03:04:05Z",
            "managedFields": [{"manager": "kubectl", "operation": "Update", "apiVersion": "v1"}],
        },
        "spec": {"nodeName": "node-1", "containers": [{"name": "c", "image": "busybox"}]},
        "status": {"phase": "Running"},
    })
}

/// A `PodList` page.
pub(super) fn pod_list(items: &[&str], continue_token: Option<&str>, rv: &str) -> Value {
    let mut metadata = json!({"resourceVersion": rv});
    if let Some(token) = continue_token {
        metadata["continue"] = json!(token);
    }
    json!({
        "kind": "PodList", "apiVersion": "v1", "metadata": metadata,
        "items": items.iter().map(|n| pod_item(n)).collect::<Vec<_>>(),
    })
}

pub(super) fn gone() -> Value {
    status_body(
        410,
        "Expired",
        "The provided continue parameter is too old to display a consistent list result.",
    )
}

/// The decoded query of the `n`th request that hit `path`, as `key=value&key=value` in
/// request order.
pub(super) fn query_of(api: &FakeApi, path: &str, n: usize) -> String {
    let raw = api
        .requests()
        .into_iter()
        .filter(|r| r.path == path)
        .nth(n)
        .unwrap_or_else(|| panic!("no request #{n} to {path}"))
        .query;
    url::form_urlencoded::parse(raw.as_bytes())
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// `list` of default-namespace pods with `options`.
pub(super) async fn list_pods(
    r: &KubeResources,
    options: oxikube_ports::ListOptions,
) -> oxikube_domain::OxiResult<oxikube_ports::ListPage> {
    oxikube_ports::ResourceReader::list(r, &pod_gvk(), Some("default"), &options).await
}
