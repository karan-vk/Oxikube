//! Fixtures: a fake API server whose discovery serves `ConfigMap`, `Namespace`, the `Widget`
//! CRD and a create-only `Binding`.

use serde_json::{Value, json};

use crate::discovery::{DiscoveryConfig, KubeDiscovery};
use crate::fake_api::FakeApi;
use crate::resources::KubeResources;
use oxikube_domain::ids::Gvk;
use oxikube_ports::WriteOptions;

pub(super) const CONFIGMAPS: &str = "/api/v1/namespaces/default/configmaps";
pub(super) const CONFIGMAP_A: &str = "/api/v1/namespaces/default/configmaps/a";
pub(super) const WIDGET_A: &str = "/apis/test.oxikube.dev/v1/namespaces/default/widgets/a";

pub(super) fn configmap_gvk() -> Gvk {
    Gvk::new("", "v1", "ConfigMap")
}

pub(super) fn namespace_gvk() -> Gvk {
    Gvk::new("", "v1", "Namespace")
}

pub(super) fn widget_gvk() -> Gvk {
    Gvk::new("test.oxikube.dev", "v1", "Widget")
}

pub(super) fn binding_gvk() -> Gvk {
    Gvk::new("", "v1", "Binding")
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
        "get",
        "list",
        "watch",
        "create",
        "update",
        "patch",
        "delete",
        "deletecollection",
    ];
    api.reply(
        "/api",
        200,
        json!({"kind": "APIVersions", "versions": ["v1"]}),
    );
    api.reply(
        "/apis",
        200,
        json!({"kind": "APIGroupList", "groups": [group("test.oxikube.dev", "v1")]}),
    );
    api.reply(
        "/api/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "v1", "resources": [
            resource("configmaps", "ConfigMap", true, &all),
            resource("namespaces", "Namespace", false, &all),
            resource("bindings", "Binding", true, &["create"]),
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

/// A ConfigMap as the server returns it after a write.
pub(super) fn configmap(name: &str, rv: &str) -> Value {
    json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": name, "namespace": "default", "uid": format!("uid-{name}"),
                     "resourceVersion": rv},
        "data": {"k": "v"},
    })
}

/// A manifest as a caller would send it: no `apiVersion`, `kind` or server fields.
pub(super) fn manifest(name: &str) -> Value {
    json!({"metadata": {"name": name}, "data": {"k": "v"}})
}

/// The query of the only request to `path`, decoded and sorted.
pub(super) fn query_of(api: &FakeApi, path: &str) -> Vec<String> {
    let requests: Vec<_> = api
        .requests()
        .into_iter()
        .filter(|r| r.path == path)
        .collect();
    assert_eq!(requests.len(), 1, "expected one request to {path}");
    let mut pairs: Vec<String> = url::form_urlencoded::parse(requests[0].query.as_bytes())
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    pairs.sort();
    pairs
}

/// The last request recorded for `path`.
pub(super) fn last_to(api: &FakeApi, path: &str) -> crate::fake_api::Recorded {
    api.requests()
        .into_iter()
        .rfind(|r| r.path == path)
        .unwrap_or_else(|| panic!("no request to {path}"))
}

pub(super) fn no_options() -> WriteOptions {
    WriteOptions::default()
}
