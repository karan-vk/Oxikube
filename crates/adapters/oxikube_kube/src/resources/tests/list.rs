//! Single-page lists: selectors, resourceVersion, scoping, metadata-only.

use oxikube_domain::ErrorKind;
use oxikube_ports::{ListOptions, ResourceReader, VersionMatch};
use serde_json::json;

use super::harness::*;

#[tokio::test]
async fn a_page_carries_items_token_and_list_resource_version() {
    let api = server();
    let mut page = pod_list(&["a", "b"], Some("next"), "777");
    page["metadata"]["remainingItemCount"] = json!(40);
    api.reply(PODS, 200, page);

    let page = resources(&api)
        .list(
            &pod_gvk(),
            Some("default"),
            &ListOptions::default().limit(2),
        )
        .await
        .unwrap();

    assert_eq!(page.items.len(), 2);
    assert_eq!(page.continue_token.as_deref(), Some("next"));
    assert_eq!(page.resource_version.as_deref(), Some("777"));
    assert_eq!(page.remaining_item_count, Some(40));
    assert!(page.has_more());
}

#[tokio::test]
async fn label_and_field_selectors_reach_the_server() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a"], None, "1"));
    resources(&api)
        .list(
            &pod_gvk(),
            Some("default"),
            &ListOptions::default()
                .labels("app=web,tier!=db")
                .fields("spec.nodeName=node-1"),
        )
        .await
        .unwrap();
    let query = query_of(&api, PODS, 0);
    assert!(query.contains("labelSelector=app=web,tier!=db"), "{query}");
    assert!(
        query.contains("fieldSelector=spec.nodeName=node-1"),
        "{query}"
    );
}

#[tokio::test]
async fn resource_version_semantics_reach_the_server() {
    let api = server();
    api.reply(PODS, 200, pod_list(&[], None, "1"));
    let r = resources(&api);
    let list = |options: ListOptions| list_pods(&r, options);

    list(ListOptions::default()).await.unwrap();
    assert_eq!(
        query_of(&api, PODS, 0),
        "",
        "unset: consistent read, nothing sent"
    );

    list(ListOptions::default().at("42", VersionMatch::Exact))
        .await
        .unwrap();
    assert_eq!(
        query_of(&api, PODS, 1),
        "resourceVersion=42&resourceVersionMatch=Exact"
    );

    list(ListOptions::default().at("42", VersionMatch::NotOlderThan))
        .await
        .unwrap();
    assert_eq!(
        query_of(&api, PODS, 2),
        "resourceVersion=42&resourceVersionMatch=NotOlderThan"
    );

    let mut any = ListOptions::default();
    any.resource_version = Some("0".into());
    list(any.clone()).await.unwrap();
    assert_eq!(
        query_of(&api, PODS, 3),
        "resourceVersion=0",
        "any cached version"
    );

    // With a limit, "0" would make the server ignore the limit, so it is not sent.
    list(any.limit(10)).await.unwrap();
    assert_eq!(query_of(&api, PODS, 4), "limit=10");

    // A continuation never repeats the resource version.
    list(
        ListOptions::default()
            .at("42", VersionMatch::NotOlderThan)
            .continue_from("tok"),
    )
    .await
    .unwrap();
    assert_eq!(query_of(&api, PODS, 5), "continue=tok");
}

#[tokio::test]
async fn invalid_resource_version_combinations_never_reach_the_server() {
    let api = server();
    let mut options = ListOptions::default();
    options.version_match = Some(VersionMatch::Exact);
    let err = resources(&api)
        .list(&pod_gvk(), Some("default"), &options)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert_eq!(api.hits(PODS), 0);
}

#[tokio::test]
async fn namespaced_kinds_list_one_namespace_or_all() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a"], None, "1"));
    api.reply(ALL_PODS, 200, pod_list(&["a", "b"], None, "1"));
    let r = resources(&api);

    let one = r
        .list(&pod_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap();
    let all = r
        .list(&pod_gvk(), None, &ListOptions::default())
        .await
        .unwrap();
    let blank = r
        .list(&pod_gvk(), Some(""), &ListOptions::default())
        .await
        .unwrap();

    assert_eq!(
        (one.items.len(), all.items.len(), blank.items.len()),
        (1, 2, 2)
    );
}

#[tokio::test]
async fn cluster_scoped_kinds_list_without_a_namespace() {
    let api = server();
    api.reply(
        "/api/v1/namespaces",
        200,
        json!({"kind": "NamespaceList", "apiVersion": "v1", "metadata": {"resourceVersion": "3"},
               "items": [{"metadata": {"name": "default"}}, {"metadata": {"name": "kube-system"}}]}),
    );
    let page = resources(&api)
        .list(&namespace_gvk(), None, &ListOptions::default())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].kind, namespace_gvk());
    assert_eq!(page.items[0].namespace(), None);

    let err = resources(&api)
        .list(&namespace_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[tokio::test]
async fn list_metadata_returns_object_meta() {
    let api = server();
    api.reply(
        PODS,
        200,
        json!({"kind": "PartialObjectMetadataList", "apiVersion": "meta.k8s.io/v1",
               "metadata": {"resourceVersion": "9", "continue": "c"},
               "items": [{"metadata": {"name": "a", "namespace": "default", "labels": {"app": "web"},
                                       "managedFields": [{"manager": "m"}]}}]}),
    );
    let page = resources(&api)
        .list_metadata(
            &pod_gvk(),
            Some("default"),
            &ListOptions::default().limit(1),
        )
        .await
        .unwrap();
    assert_eq!(&*page.items[0].name, "a");
    assert_eq!(page.items[0].labels.get("app").map(|v| &**v), Some("web"));
    assert_eq!(page.continue_token.as_deref(), Some("c"));
    assert_eq!(page.resource_version.as_deref(), Some("9"));
}

#[tokio::test]
async fn deferred_methods_are_unsupported() {
    let api = server();
    let r = resources(&api);
    let err = r
        .watch(&pod_gvk(), None, &oxikube_ports::WatchOptions::default())
        .await
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert_eq!(
        r.get_scale(&pod_gvk(), Some("default"), "a")
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
}
