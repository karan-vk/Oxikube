//! Server JSON to domain `Resource`: type fields, `managedFields`, CRDs.

use oxikube_ports::{ListOptions, ResourceReader};
use serde_json::json;

use super::harness::*;
use crate::resources::{ManagedFields, ResourcesConfig};

const POD_A: &str = "/api/v1/namespaces/default/pods/a";

fn full_pod() -> serde_json::Value {
    let mut pod = pod_item("a");
    pod["apiVersion"] = "v1".into();
    pod["kind"] = "Pod".into();
    pod
}

#[tokio::test]
async fn list_items_get_their_type_fields_from_discovery() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a"], None, "1"));
    let page = resources(&api)
        .list(&pod_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap();
    let pod = &page.items[0];
    assert_eq!(pod.kind, pod_gvk());
    assert_eq!(pod.json["apiVersion"], "v1");
    assert_eq!(pod.json["kind"], "Pod");
    assert_eq!(pod.namespace(), Some("default"));
    assert_eq!(pod.get_str("/spec/nodeName"), Some("node-1"));
    assert_eq!(&**pod.meta.uid.as_ref().unwrap(), "uid-a");
}

#[tokio::test]
async fn lists_strip_managed_fields_and_get_keeps_them() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a"], None, "1"));
    api.reply(POD_A, 200, full_pod());
    let r = resources(&api);

    let listed = r
        .list(&pod_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap();
    assert!(listed.items[0].get("/metadata/managedFields").is_none());

    let got = r.get(&pod_gvk(), Some("default"), "a").await.unwrap();
    assert!(got.get("/metadata/managedFields/0/manager").is_some());
}

#[tokio::test]
async fn managed_fields_handling_is_configurable() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a"], None, "1"));
    api.reply(POD_A, 200, full_pod());
    let config = ResourcesConfig {
        list_managed_fields: ManagedFields::Keep,
        get_managed_fields: ManagedFields::Strip,
        ..ResourcesConfig::default()
    };
    let r = resources_with(&api, config);

    let listed = r
        .list(&pod_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap();
    assert!(listed.items[0].get("/metadata/managedFields").is_some());
    let got = r.get(&pod_gvk(), Some("default"), "a").await.unwrap();
    assert!(got.get("/metadata/managedFields").is_none());
}

#[tokio::test]
async fn key_order_is_type_fields_metadata_then_the_rest() {
    let api = server();
    api.reply(POD_A, 200, full_pod());
    let got = resources(&api)
        .get(&pod_gvk(), Some("default"), "a")
        .await
        .unwrap();
    let keys: Vec<_> = got.json.as_object().unwrap().keys().cloned().collect();
    assert_eq!(&keys[..3], ["apiVersion", "kind", "metadata"]);
    assert!(keys.contains(&"spec".to_owned()) && keys.contains(&"status".to_owned()));
}

#[tokio::test]
async fn custom_resources_keep_every_field() {
    let api = server();
    let widget = json!({
        "apiVersion": "test.oxikube.dev/v1", "kind": "Widget",
        "metadata": {"name": "w1", "namespace": "default", "resourceVersion": "5",
                     "finalizers": ["test.oxikube.dev/guard"]},
        "spec": {"size": 3, "tags": ["a", "b"], "nested": {"deep": {"x": null}}},
        "status": {"phase": "Ready"},
    });
    api.reply(
        "/apis/test.oxikube.dev/v1/namespaces/default/widgets",
        200,
        json!({"kind": "WidgetList", "apiVersion": "test.oxikube.dev/v1",
               "metadata": {"resourceVersion": "5"}, "items": [widget.clone()]}),
    );
    let page = resources(&api)
        .list(&widget_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap();
    assert_eq!(page.items[0].kind, widget_gvk());
    assert_eq!(*page.items[0].json, widget);
    assert_eq!(page.items[0].meta.finalizers.len(), 1);
}

#[tokio::test]
async fn an_unversioned_gvk_resolves_to_the_preferred_version() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a"], None, "1"));
    let page = resources(&api)
        .list(
            &oxikube_domain::ids::Gvk::new("", "", "Pod"),
            Some("default"),
            &ListOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(page.items[0].kind, pod_gvk());
}

#[tokio::test]
async fn an_object_without_a_name_is_an_internal_error() {
    let api = server();
    api.reply(
        PODS,
        200,
        json!({"kind": "PodList", "apiVersion": "v1", "metadata": {}, "items": [{"metadata": {}}]}),
    );
    let err = resources(&api)
        .list(&pod_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), oxikube_domain::ErrorKind::Internal);
}
