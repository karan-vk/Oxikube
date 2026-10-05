//! Patch kinds on the wire: content type, body, `force`, field manager, dry run.

use http::Method;
use oxikube_domain::ErrorKind;
use oxikube_ports::{Patch, ResourceWriter, WriteOptions};
use serde_json::json;

use super::harness::*;

async fn send(
    api: &crate::fake_api::FakeApi,
    patch: Patch,
    options: WriteOptions,
) -> oxikube_domain::OxiResult<oxikube_domain::Resource> {
    resources(api)
        .patch(&configmap_gvk(), Some("default"), "a", &patch, &options)
        .await
}

#[tokio::test]
async fn each_patch_kind_uses_its_content_type() {
    let ops = json!([{"op": "replace", "path": "/data/k", "value": "w"}]);
    let object = json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "a"}, "data": {"k": "w"}});
    let cases = [
        (
            Patch::merge(json!({"data": {"k": "w"}})),
            "application/merge-patch+json",
        ),
        (
            Patch::strategic(json!({"data": {"k": "w"}})),
            "application/strategic-merge-patch+json",
        ),
        (Patch::json(ops), "application/json-patch+json"),
        (
            Patch::apply(object, "oxikube", false),
            "application/apply-patch+yaml",
        ),
    ];
    for (patch, content_type) in cases {
        let api = server();
        api.reply(CONFIGMAP_A, 200, configmap("a", "51"));
        let body = patch.body.clone();
        let patched = send(&api, patch, WriteOptions::default())
            .await
            .expect("patch");
        assert_eq!(patched.meta.resource_version.as_deref(), Some("51"));
        let sent = last_to(&api, CONFIGMAP_A);
        assert_eq!(sent.method, Method::PATCH);
        assert_eq!(sent.content_type.as_deref(), Some(content_type));
        assert_eq!(sent.body, Some(body), "{content_type}");
    }
}

#[tokio::test]
async fn apply_sends_force_only_when_asked_and_names_the_manager() {
    let object = json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "a"}});

    let api = server();
    api.reply(CONFIGMAP_A, 200, configmap("a", "1"));
    send(
        &api,
        Patch::apply(object.clone(), "oxikube", false),
        WriteOptions::default(),
    )
    .await
    .expect("apply");
    assert_eq!(query_of(&api, CONFIGMAP_A), ["fieldManager=oxikube"]);

    let api = server();
    api.reply(CONFIGMAP_A, 200, configmap("a", "1"));
    send(
        &api,
        Patch::apply(object.clone(), "oxikube", true),
        WriteOptions::default(),
    )
    .await
    .expect("forced apply");
    assert_eq!(
        query_of(&api, CONFIGMAP_A),
        ["fieldManager=oxikube", "force=true"]
    );

    // The patch's own manager wins; an empty one falls back to the options, then the default.
    let api = server();
    api.reply(CONFIGMAP_A, 200, configmap("a", "1"));
    send(
        &api,
        Patch::apply(object.clone(), "helm", false),
        WriteOptions::dry_run().manager("other"),
    )
    .await
    .expect("dry-run apply");
    assert_eq!(
        query_of(&api, CONFIGMAP_A),
        ["dryRun=All", "fieldManager=helm"]
    );

    let api = server();
    api.reply(CONFIGMAP_A, 200, configmap("a", "1"));
    send(
        &api,
        Patch::apply(object, "", false),
        WriteOptions::default().manager("other"),
    )
    .await
    .expect("apply");
    assert_eq!(query_of(&api, CONFIGMAP_A), ["fieldManager=other"]);
}

#[tokio::test]
async fn non_apply_patches_never_send_force_and_still_name_a_manager() {
    let api = server();
    api.reply(CONFIGMAP_A, 200, configmap("a", "1"));
    send(
        &api,
        Patch::merge(json!({"data": {}})),
        WriteOptions::dry_run(),
    )
    .await
    .expect("patch");
    assert_eq!(
        query_of(&api, CONFIGMAP_A),
        ["dryRun=All", "fieldManager=oxikube"]
    );
}

#[tokio::test]
async fn a_json_patch_must_be_an_operation_array() {
    let api = server();
    let err = send(
        &api,
        Patch::json(json!({"op": "add"})),
        WriteOptions::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert_eq!(api.hits(CONFIGMAP_A), 0);
}
