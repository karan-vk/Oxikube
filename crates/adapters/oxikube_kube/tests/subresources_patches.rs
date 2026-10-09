//! Kind integration for E04-S06 (patch builders): rollout restart, cordon and uncordon, cronjob
//! suspend and resume, applied through `ResourceWriter::patch`. Cordon runs on a fake Node
//! object (tainted so nothing schedules there), never on the real node. Needs
//! `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use jiff::Timestamp;
use oxikube_kube::{RESTARTED_AT_ANNOTATION, ResourcePatch};
use oxikube_ports::{ResourceReader, ResourceWriter, WriteOptions};

use common::subresources::{
    FakeNode, cronjob, cronjob_gvk, deployment, deployment_gvk, node_gvk, setup,
};

#[tokio::test]
async fn rollout_restart_stamps_the_template_and_bumps_the_generation() {
    let Some(env) = setup().await else { return };
    let (r, ns) = (&env.resources, Some(env.namespace()));
    let write = WriteOptions::default();
    let created = r
        .create(&deployment_gvk(), ns, &deployment("web", 0), &write)
        .await
        .expect("create");
    let generation = created.to_value()["metadata"]["generation"].as_i64();

    let at: Timestamp = "2026-10-06T12:34:56Z".parse().expect("timestamp");
    let restarted = r
        .patch(
            &deployment_gvk(),
            ns,
            "web",
            &ResourcePatch::RolloutRestart { at }.to_patch(),
            &write,
        )
        .await
        .expect("restart");
    assert_eq!(
        restarted.to_value()["spec"]["template"]["metadata"]["annotations"]
            [RESTARTED_AT_ANNOTATION],
        "2026-10-06T12:34:56Z"
    );
    assert!(
        restarted.to_value()["metadata"]["generation"].as_i64() > generation,
        "a changed pod template is a new generation"
    );
    // Other template fields survive the merge.
    assert_eq!(
        restarted.to_value()["spec"]["template"]["spec"]["containers"][0]["name"],
        "pause"
    );
}

#[tokio::test]
async fn cordon_and_uncordon_flip_unschedulable_on_a_node() {
    let Some(env) = setup().await else { return };
    let node = FakeNode::create(&env.client, &env.context).await;
    let r = &env.resources;
    let write = WriteOptions::default();

    let before = r
        .get(&node_gvk(), None, &node.name)
        .await
        .expect("get node");
    assert!(before.to_value()["spec"]["unschedulable"].is_null());

    let cordoned = r
        .patch(
            &node_gvk(),
            None,
            &node.name,
            &ResourcePatch::Cordon.to_patch(),
            &write,
        )
        .await
        .expect("cordon");
    assert_eq!(cordoned.to_value()["spec"]["unschedulable"], true);

    // A dry-run uncordon answers with the result and leaves the node cordoned.
    let dry = r
        .patch(
            &node_gvk(),
            None,
            &node.name,
            &ResourcePatch::Uncordon.to_patch(),
            &WriteOptions::dry_run(),
        )
        .await
        .expect("dry-run uncordon");
    assert_ne!(dry.to_value()["spec"]["unschedulable"], true);
    let still = r
        .get(&node_gvk(), None, &node.name)
        .await
        .expect("get node");
    assert_eq!(still.to_value()["spec"]["unschedulable"], true);

    let uncordoned = r
        .patch(
            &node_gvk(),
            None,
            &node.name,
            &ResourcePatch::Uncordon.to_patch(),
            &write,
        )
        .await
        .expect("uncordon");
    assert_ne!(uncordoned.to_value()["spec"]["unschedulable"], true);
}

#[tokio::test]
async fn a_cronjob_is_suspended_and_resumed() {
    let Some(env) = setup().await else { return };
    let (r, ns) = (&env.resources, Some(env.namespace()));
    let write = WriteOptions::default();
    r.create(&cronjob_gvk(), ns, &cronjob("tick"), &write)
        .await
        .expect("create cronjob");

    let suspended = r
        .patch(
            &cronjob_gvk(),
            ns,
            "tick",
            &ResourcePatch::CronJobSuspend(true).to_patch(),
            &write,
        )
        .await
        .expect("suspend");
    assert_eq!(suspended.to_value()["spec"]["suspend"], true);

    let resumed = r
        .patch(
            &cronjob_gvk(),
            ns,
            "tick",
            &ResourcePatch::CronJobSuspend(false).to_patch(),
            &write,
        )
        .await
        .expect("resume");
    assert_eq!(resumed.to_value()["spec"]["suspend"], false);
    assert_eq!(resumed.to_value()["spec"]["schedule"], "0 0 29 2 1");
}
