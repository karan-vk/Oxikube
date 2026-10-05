//! `rollout_history` and `rollout_undo` on the testkit's in-memory port: revision selection
//! over several ReplicaSets, and the patch that is sent.

use oxikube_domain::ErrorKind;
use oxikube_ports::{PatchKind, WriteOptions};
use oxikube_testkit::{FakeResourcePort, ResourceCall};
use serde_json::{Value, json};

use super::harness::*;
use crate::algorithms::{RolloutRevision, rollout_history, rollout_undo};

/// Deployment `web` at revision 3 (image v3), with ReplicaSets for revisions 1 to 3 (inserted
/// out of order), one of another Deployment and one without a revision.
fn cluster() -> FakeResourcePort {
    let port = FakeResourcePort::new().with_objects([
        object(deployment_json(3, "web:v3")),
        object(replicaset_json(
            "web-c",
            "d-uid",
            3,
            "web:v3",
            json!({"kubernetes.io/change-cause": "bump to v3"}),
        )),
        object(replicaset_json("web-a", "d-uid", 1, "web:v1", json!({}))),
        object(replicaset_json(
            "web-b",
            "d-uid",
            2,
            "web:v2",
            json!({
                "kubernetes.io/change-cause": "bump to v2",
                "kubectl.kubernetes.io/last-applied-configuration": "{}",
                "deployment.kubernetes.io/desired-replicas": "2",
                "deployment.kubernetes.io/max-replicas": "3",
                "team": "web",
            }),
        )),
        // Same labels, another Deployment's: never part of this history.
        object(replicaset_json(
            "other-x",
            "other-uid",
            9,
            "other:v9",
            json!({}),
        )),
    ]);
    let mut unversioned = replicaset_json("web-n", "d-uid", 1, "web:v0", json!({}));
    unversioned["metadata"]["annotations"] = json!({});
    port.insert(object(unversioned));
    port
}

fn patch_of(port: &FakeResourcePort) -> (oxikube_ports::Patch, WriteOptions) {
    let mut patches = port
        .mutating_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Patch { patch, options, .. } => Some((patch, options)),
            _ => None,
        });
    let found = patches.next().expect("a patch was sent");
    assert!(patches.next().is_none(), "exactly one patch");
    found
}

#[tokio::test]
async fn history_lists_the_revisions_oldest_first_with_cause_and_images() {
    let history = rollout_history(&cluster(), "default", "web")
        .await
        .expect("history");
    let summary: Vec<_> = history
        .iter()
        .map(|r: &RolloutRevision| {
            (
                r.revision,
                r.replica_set.as_str(),
                r.change_cause.as_deref(),
                r.images.clone(),
                r.current,
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (1, "web-a", None, vec!["web:v1".to_owned()], false),
            (
                2,
                "web-b",
                Some("bump to v2"),
                vec!["web:v2".to_owned()],
                false
            ),
            (
                3,
                "web-c",
                Some("bump to v3"),
                vec!["web:v3".to_owned()],
                true
            ),
        ]
    );
    assert!(history[0].created.is_some());
}

#[tokio::test]
async fn revisions_are_ordered_as_numbers_not_as_text() {
    let mut objects = vec![object(deployment_json(10, "web:v10"))];
    for revision in [9, 10, 2] {
        objects.push(object(replicaset_json(
            &format!("web-{revision}"),
            "d-uid",
            revision,
            &format!("web:v{revision}"),
            json!({}),
        )));
    }
    let port = FakeResourcePort::new().with_objects(objects);
    let revisions: Vec<i64> = rollout_history(&port, "default", "web")
        .await
        .expect("history")
        .iter()
        .map(|r| r.revision)
        .collect();
    assert_eq!(revisions, vec![2, 9, 10]);

    // The previous revision of 10 is 9, not 2.
    port.script()
        .patch
        .push_ok(object(deployment_json(10, "web:v9")));
    let undo = rollout_undo(&port, "default", "web", None, &WriteOptions::default())
        .await
        .expect("undo");
    assert_eq!(undo.to_revision, 9);
}

#[tokio::test]
async fn history_of_a_missing_deployment_is_not_found() {
    let err = rollout_history(&FakeResourcePort::new(), "default", "web")
        .await
        .expect_err("missing");
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn undo_restores_the_previous_revisions_template_with_a_json_patch() {
    let port = cluster();
    port.script()
        .patch
        .push_ok(object(deployment_json(4, "web:v2")));
    let undo = rollout_undo(&port, "default", "web", None, &WriteOptions::default())
        .await
        .expect("undo");
    assert_eq!(
        (undo.from_revision, undo.to_revision, undo.unchanged),
        (Some(3), 2, false)
    );
    assert!(undo.deployment.is_some());

    let (patch, options) = patch_of(&port);
    assert_eq!(patch.kind, PatchKind::Json);
    assert!(!options.dry_run);
    assert_eq!(
        patch.body,
        json!([
            // The ReplicaSet's template, minus its `pod-template-hash` label.
            {"op": "replace", "path": "/spec/template", "value": {
                "metadata": {"labels": {"app": "web"}},
                "spec": {"containers": [{"name": "web", "image": "web:v2"}]},
            }},
            // Its annotations, minus the revision bookkeeping and the last-applied copy.
            {"op": "add", "path": "/metadata/annotations", "value": {
                "kubernetes.io/change-cause": "bump to v2",
                "team": "web",
            }},
        ])
    );
}

#[tokio::test]
async fn undo_to_a_named_revision_picks_that_replicaset() {
    let port = cluster();
    port.script()
        .patch
        .push_ok(object(deployment_json(4, "web:v1")));
    let undo = rollout_undo(&port, "default", "web", Some(1), &WriteOptions::default())
        .await
        .expect("undo");
    assert_eq!(undo.to_revision, 1);
    let (patch, _) = patch_of(&port);
    assert_eq!(
        patch.body[0]["value"]["spec"]["containers"][0]["image"],
        Value::from("web:v1")
    );
}

#[tokio::test]
async fn undo_to_the_template_already_running_changes_nothing() {
    let port = cluster();
    let undo = rollout_undo(&port, "default", "web", Some(3), &WriteOptions::default())
        .await
        .expect("undo");
    assert_eq!((undo.to_revision, undo.unchanged), (3, true));
    assert!(undo.deployment.is_none());
    assert!(port.mutating_calls().is_empty());
}

#[tokio::test]
async fn a_dry_run_reaches_the_patch() {
    let port = cluster();
    port.script()
        .patch
        .push_ok(object(deployment_json(3, "web:v2")));
    rollout_undo(&port, "default", "web", None, &WriteOptions::dry_run())
        .await
        .expect("undo");
    assert!(patch_of(&port).1.dry_run);
}

#[tokio::test]
async fn undo_refuses_what_kubectl_refuses() {
    let port = cluster();
    let options = WriteOptions::default();
    let go = |to| rollout_undo(&port, "default", "web", to, &options);
    for bad in [Some(0), Some(-2)] {
        assert_eq!(
            go(bad).await.expect_err("bad").kind(),
            ErrorKind::Validation
        );
    }
    let missing = go(Some(7)).await.expect_err("unknown revision");
    assert_eq!(missing.kind(), ErrorKind::Validation);
    assert!(missing.to_string().contains("revision 7"), "{missing}");
    assert!(port.mutating_calls().is_empty());

    // One revision only: nothing to go back to.
    let single = FakeResourcePort::new().with_objects([
        object(deployment_json(1, "web:v1")),
        object(replicaset_json("web-a", "d-uid", 1, "web:v1", json!({}))),
    ]);
    let err = rollout_undo(&single, "default", "web", None, &WriteOptions::default())
        .await
        .expect_err("no history");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.to_string().contains("no rollout history"), "{err}");

    // A paused Deployment must be resumed first.
    let mut paused = deployment_json(3, "web:v3");
    paused["spec"]["paused"] = json!(true);
    let port = cluster();
    port.insert(object(paused));
    let err = rollout_undo(&port, "default", "web", None, &WriteOptions::default())
        .await
        .expect_err("paused");
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(port.mutating_calls().is_empty());
}
