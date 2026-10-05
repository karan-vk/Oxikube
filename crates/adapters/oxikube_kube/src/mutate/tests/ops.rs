//! create, replace, delete and delete-collection requests and their results.

use http::Method;
use oxikube_domain::ErrorKind;
use oxikube_ports::{
    DeleteCollectionOutcome, DeleteOptions, DeleteOutcome, ListOptions, Preconditions,
    PropagationPolicy, ResourceWriter, WriteOptions,
};
use serde_json::{Value, json};

use super::harness::*;
use crate::fake_api::status_body;

#[tokio::test]
async fn create_posts_to_the_collection_and_fills_the_type_fields() {
    let api = server();
    api.reply(CONFIGMAPS, 201, configmap("a", "11"));
    let created = resources(&api)
        .create(
            &configmap_gvk(),
            Some("default"),
            &manifest("a"),
            &no_options(),
        )
        .await
        .expect("create");

    assert_eq!(created.name(), "a");
    assert_eq!(created.meta.resource_version.as_deref(), Some("11"));
    let sent = last_to(&api, CONFIGMAPS);
    assert_eq!(sent.method, Method::POST);
    let body = sent.body.expect("body");
    assert_eq!(body["apiVersion"], "v1");
    assert_eq!(body["kind"], "ConfigMap");
    assert_eq!(body["data"]["k"], "v");
    assert_eq!(query_of(&api, CONFIGMAPS), ["fieldManager=oxikube"]);
}

#[tokio::test]
async fn dry_run_sets_the_query_and_returns_the_servers_object() {
    let api = server();
    api.reply(CONFIGMAPS, 201, configmap("a", "12"));
    let opts = WriteOptions::dry_run().manager("editor");
    let preview = resources(&api)
        .create(&configmap_gvk(), Some("default"), &manifest("a"), &opts)
        .await
        .expect("dry-run create");
    assert_eq!(preview.meta.uid.as_deref(), Some("uid-a"));
    assert_eq!(
        query_of(&api, CONFIGMAPS),
        ["dryRun=All", "fieldManager=editor"]
    );
}

#[tokio::test]
async fn replace_puts_the_body_with_its_resource_version() {
    let api = server();
    api.reply(CONFIGMAP_A, 200, configmap("a", "21"));
    let mut object = manifest("a");
    object["metadata"]["resourceVersion"] = json!("20");
    let replaced = resources(&api)
        .replace(
            &configmap_gvk(),
            Some("default"),
            "a",
            &object,
            &no_options(),
        )
        .await
        .expect("replace");
    assert_eq!(replaced.meta.resource_version.as_deref(), Some("21"));
    let sent = last_to(&api, CONFIGMAP_A);
    assert_eq!(sent.method, Method::PUT);
    assert_eq!(
        sent.body.expect("body")["metadata"]["resourceVersion"],
        "20"
    );
}

#[tokio::test]
async fn custom_resources_take_the_same_path() {
    let api = server();
    let widget = json!({"apiVersion": "test.oxikube.dev/v1", "kind": "Widget",
        "metadata": {"name": "a", "namespace": "default"}, "spec": {"size": 3}});
    api.reply(WIDGET_A, 200, widget.clone());
    let replaced = resources(&api)
        .replace(&widget_gvk(), Some("default"), "a", &widget, &no_options())
        .await
        .expect("replace a custom resource");
    assert_eq!(replaced.json["spec"]["size"], 3);
}

#[tokio::test]
async fn bad_requests_are_rejected_before_any_request_is_sent() {
    let api = server();
    let r = resources(&api);
    let write = no_options();

    let not_object = r
        .create(&configmap_gvk(), Some("default"), &json!([1]), &write)
        .await
        .unwrap_err();
    assert_eq!(not_object.kind(), ErrorKind::Validation);

    let bad_meta = r
        .create(
            &configmap_gvk(),
            Some("default"),
            &json!({"metadata": {"labels": "not a map"}}),
            &write,
        )
        .await
        .unwrap_err();
    assert_eq!(bad_meta.kind(), ErrorKind::Validation);
    assert!(!bad_meta.message().contains("not a map"), "{bad_meta}");

    let namespaced_without = r
        .create(&configmap_gvk(), None, &manifest("a"), &write)
        .await
        .unwrap_err();
    assert_eq!(namespaced_without.kind(), ErrorKind::Validation);

    let cluster_scoped_with = r
        .create(&namespace_gvk(), Some("x"), &manifest("a"), &write)
        .await
        .unwrap_err();
    assert_eq!(cluster_scoped_with.kind(), ErrorKind::Validation);

    let too_long = WriteOptions::default().manager("m".repeat(129));
    let manager = r
        .create(&configmap_gvk(), Some("default"), &manifest("a"), &too_long)
        .await
        .unwrap_err();
    assert_eq!(manager.kind(), ErrorKind::Validation);

    let no_verb = r
        .replace(&binding_gvk(), Some("default"), "a", &manifest("a"), &write)
        .await
        .unwrap_err();
    assert_eq!(no_verb.kind(), ErrorKind::Unsupported);

    assert!(
        api.requests().iter().all(|req| req.method == Method::GET),
        "only discovery reads went out: {:?}",
        api.requests()
    );
}

fn delete_body(api: &crate::fake_api::FakeApi) -> Value {
    last_to(api, CONFIGMAP_A).body.unwrap_or(Value::Null)
}

#[tokio::test]
async fn delete_reports_gone_or_still_deleting() {
    let api = server();
    api.reply(
        CONFIGMAP_A,
        200,
        json!({"kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Success", "code": 200}),
    );
    let r = resources(&api);
    let gone = r
        .delete(
            &configmap_gvk(),
            Some("default"),
            "a",
            &DeleteOptions::default(),
        )
        .await
        .expect("delete");
    assert_eq!(gone, DeleteOutcome::Deleted);
    assert_eq!(last_to(&api, CONFIGMAP_A).method, Method::DELETE);

    let api = server();
    let mut dying = configmap("a", "30");
    dying["metadata"]["deletionTimestamp"] = json!("2026-01-02T03:04:05Z");
    api.reply(CONFIGMAP_A, 200, dying);
    let outcome = resources(&api)
        .delete(
            &configmap_gvk(),
            Some("default"),
            "a",
            &DeleteOptions::default(),
        )
        .await
        .expect("delete");
    match outcome {
        DeleteOutcome::Deleting(object) => assert!(object.meta.deletion.is_some()),
        other => panic!("expected Deleting, got {other:?}"),
    }
}

#[tokio::test]
async fn delete_sends_propagation_grace_preconditions_and_dry_run() {
    for (policy, wire) in [
        (PropagationPolicy::Foreground, "Foreground"),
        (PropagationPolicy::Background, "Background"),
        (PropagationPolicy::Orphan, "Orphan"),
    ] {
        let api = server();
        api.reply(CONFIGMAP_A, 200, configmap("a", "31"));
        let options = DeleteOptions::dry_run()
            .propagation(policy)
            .grace_period_secs(0)
            .preconditions(Preconditions {
                resource_version: Some("30".into()),
                uid: Some("uid-a".into()),
            });
        resources(&api)
            .delete(&configmap_gvk(), Some("default"), "a", &options)
            .await
            .expect("delete");
        let body = delete_body(&api);
        assert_eq!(body["propagationPolicy"], wire);
        assert_eq!(body["gracePeriodSeconds"], 0);
        assert_eq!(body["dryRun"], json!(["All"]));
        assert_eq!(body["preconditions"]["resourceVersion"], "30");
        assert_eq!(body["preconditions"]["uid"], "uid-a");
    }
}

#[tokio::test]
async fn delete_collection_sends_the_selectors_and_returns_the_doomed_objects() {
    let api = server();
    api.reply(
        CONFIGMAPS,
        200,
        json!({"kind": "ConfigMapList", "apiVersion": "v1", "metadata": {"resourceVersion": "40"},
               "items": [configmap("a", "40"), configmap("b", "40")]}),
    );
    let selection = ListOptions::default()
        .labels("app=web")
        .fields("metadata.name!=keep")
        .limit(5)
        .continue_from("ignored");
    let outcome = resources(&api)
        .delete_collection(
            &configmap_gvk(),
            Some("default"),
            &selection,
            &DeleteOptions::default(),
        )
        .await
        .expect("delete collection");
    let DeleteCollectionOutcome::Deleting(items) = outcome else {
        panic!("expected Deleting");
    };
    assert_eq!(
        items.iter().map(|r| r.name()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(last_to(&api, CONFIGMAPS).method, Method::DELETE);
    assert_eq!(
        query_of(&api, CONFIGMAPS),
        ["fieldSelector=metadata.name!=keep", "labelSelector=app=web"]
    );
}

#[tokio::test]
async fn delete_collection_status_answer_and_namespace_rule() {
    let api = server();
    api.reply(
        CONFIGMAPS,
        200,
        json!({"kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Success", "code": 200}),
    );
    let r = resources(&api);
    let outcome = r
        .delete_collection(
            &configmap_gvk(),
            Some("default"),
            &ListOptions::default().labels("a=b"),
            &DeleteOptions::default(),
        )
        .await
        .expect("delete collection");
    assert_eq!(outcome, DeleteCollectionOutcome::Deleted);

    let err = r
        .delete_collection(
            &configmap_gvk(),
            None,
            &ListOptions::default().labels("a=b"),
            &DeleteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert_eq!(api.hits(CONFIGMAPS), 1, "no cluster-wide delete went out");
}

#[tokio::test]
async fn a_missing_object_on_delete_is_not_found() {
    let api = server();
    api.reply(
        CONFIGMAP_A,
        404,
        status_body(404, "NotFound", "configmaps \"a\" not found"),
    );
    let err = resources(&api)
        .delete(
            &configmap_gvk(),
            Some("default"),
            "a",
            &DeleteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}
