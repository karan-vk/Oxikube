//! `rollout_undo` through `KubeResources`: the requests on the wire.

use http::Method;
use oxikube_ports::WriteOptions;
use serde_json::json;

use super::harness::*;
use crate::algorithms::rollout_undo;

#[tokio::test]
async fn undo_lists_by_selector_and_sends_a_json_patch_of_the_deployment() {
    let api = server();
    api.reply(DEPLOY, 200, deployment_json(2, "web:v2"));
    api.reply(
        REPLICASETS,
        200,
        json!({"apiVersion": "apps/v1", "kind": "ReplicaSetList", "metadata": {}, "items": [
            replicaset_json("web-b", "d-uid", 2, "web:v2", json!({})),
            replicaset_json("web-a", "d-uid", 1, "web:v1", json!({})),
        ]}),
    );
    api.reply(DEPLOY, 200, deployment_json(3, "web:v1"));
    let undo = rollout_undo(
        &resources(&api),
        "default",
        "web",
        None,
        &WriteOptions::default(),
    )
    .await
    .expect("undo");
    assert_eq!(undo.to_revision, 1);

    let sent = calls(&api);
    let list = sent.iter().find(|r| r.path == REPLICASETS).expect("list");
    assert_eq!(list.method, Method::GET);
    assert!(
        query(list).contains(&"labelSelector=app=web".to_owned()),
        "{:?}",
        query(list)
    );
    let patch = sent
        .iter()
        .find(|r| r.method == Method::PATCH)
        .expect("patch");
    assert_eq!(patch.path, DEPLOY);
    assert_eq!(
        patch.content_type.as_deref(),
        Some("application/json-patch+json")
    );
    let ops = patch.body.clone().expect("body");
    assert_eq!(ops[0]["op"], "replace");
    assert_eq!(ops[0]["path"], "/spec/template");
    assert_eq!(ops[0]["value"]["spec"]["containers"][0]["image"], "web:v1");
    assert_eq!(ops[1]["path"], "/metadata/annotations");
}
