//! The typed and dynamic paths behind one seam return the same `Resource`s.

use oxikube_ports::{ListOptions, ResourceReader};
use serde_json::json;

use super::harness::*;
use crate::resources::{AccessPath, ManagedFields, ResourcesConfig};

const POD_A: &str = "/api/v1/namespaces/default/pods/a";

fn config(access_path: AccessPath) -> ResourcesConfig {
    ResourcesConfig {
        access_path,
        ..ResourcesConfig::default()
    }
}

#[tokio::test]
async fn typed_and_dynamic_lists_are_identical() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a", "b"], Some("t"), "9"));
    let options = ListOptions::default().limit(2);

    let dynamic = resources_with(&api, config(AccessPath::Dynamic))
        .list(&pod_gvk(), Some("default"), &options)
        .await
        .unwrap();
    let typed = resources_with(&api, config(AccessPath::Typed))
        .list(&pod_gvk(), Some("default"), &options)
        .await
        .unwrap();

    assert_eq!(typed, dynamic);
    assert_eq!(typed.items[0].to_value()["kind"], "Pod");
}

#[tokio::test]
async fn typed_and_dynamic_gets_are_identical_with_managed_fields_kept() {
    let api = server();
    let mut pod = pod_item("a");
    pod["apiVersion"] = "v1".into();
    pod["kind"] = "Pod".into();
    api.reply(POD_A, 200, pod);
    let keep = |path| ResourcesConfig {
        get_managed_fields: ManagedFields::Keep,
        access_path: path,
        ..ResourcesConfig::default()
    };

    let dynamic = resources_with(&api, keep(AccessPath::Dynamic))
        .get(&pod_gvk(), Some("default"), "a")
        .await
        .unwrap();
    let typed = resources_with(&api, keep(AccessPath::Typed))
        .get(&pod_gvk(), Some("default"), "a")
        .await
        .unwrap();

    assert_eq!(typed, dynamic);
    assert!(typed.get("/metadata/managedFields").is_some());
}

#[tokio::test]
async fn the_typed_path_strips_managed_fields_too() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a"], None, "1"));
    let page = resources_with(&api, config(AccessPath::Typed))
        .list(&pod_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap();
    assert!(page.items[0].get("/metadata/managedFields").is_none());
}

#[tokio::test]
async fn kinds_without_a_bundled_type_fall_back_to_dynamic() {
    let api = server();
    let widget = json!({"apiVersion": "test.oxikube.dev/v1", "kind": "Widget",
                        "metadata": {"name": "w1", "namespace": "default"}, "spec": {"size": 3}});
    api.reply(
        "/apis/test.oxikube.dev/v1/namespaces/default/widgets",
        200,
        json!({"kind": "WidgetList", "apiVersion": "test.oxikube.dev/v1", "metadata": {}, "items": [widget.clone()]}),
    );
    let page = resources_with(&api, config(AccessPath::Typed))
        .list(&widget_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap();
    assert_eq!(page.items[0].to_value(), widget);
}

#[tokio::test]
async fn typed_cluster_scoped_and_all_namespace_lists_work() {
    let api = server();
    api.reply(
        "/api/v1/namespaces",
        200,
        json!({"kind": "NamespaceList", "apiVersion": "v1", "metadata": {},
               "items": [{"metadata": {"name": "default"}}]}),
    );
    api.reply(ALL_PODS, 200, pod_list(&["a"], None, "1"));
    let r = resources_with(&api, config(AccessPath::Typed));
    let namespaces = r
        .list(&namespace_gvk(), None, &ListOptions::default())
        .await
        .unwrap();
    assert_eq!(namespaces.items[0].kind, namespace_gvk());
    let pods = r
        .list(&pod_gvk(), None, &ListOptions::default())
        .await
        .unwrap();
    assert_eq!(pods.items.len(), 1);
}
