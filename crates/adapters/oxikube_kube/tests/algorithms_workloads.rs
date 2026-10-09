//! Kind integration for E04-S07 (workload algorithms): `trigger_cronjob`, `rollout_history` and
//! `rollout_undo`. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly
//! otherwise. Everything lives in the test's `oxi-test-<rand>` namespace; the Deployments run
//! zero replicas, so no pod is scheduled.
#![cfg(feature = "integration")]

mod common;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;
use oxikube_kube::{rollout_history, rollout_undo, trigger_cronjob};
use oxikube_ports::{Patch, ResourceReader, ResourceWriter, WriteOptions};
use oxikube_testkit::images;
use serde_json::json;

use common::subresources::{Env, cronjob, cronjob_gvk, deployment, deployment_gvk, setup};
use common::wait_until;

const V1: &str = images::PAUSE;
const V2: &str = images::PAUSE_PREVIOUS;

fn job_gvk() -> Gvk {
    Gvk::new("batch", "v1", "Job")
}

async fn deployed_image(env: &Env, name: &str) -> String {
    let live = env
        .resources
        .get(&deployment_gvk(), Some(env.namespace()), name)
        .await
        .expect("get deployment");
    live.to_value()["spec"]["template"]["spec"]["containers"][0]["image"]
        .as_str()
        .expect("image")
        .to_owned()
}

#[tokio::test]
async fn triggering_a_cronjob_creates_an_owned_job_from_its_template() {
    let Some(env) = setup().await else { return };
    let ns = Some(env.namespace());
    let created = env
        .resources
        .create(
            &cronjob_gvk(),
            ns,
            &cronjob("nightly"),
            &WriteOptions::default(),
        )
        .await
        .expect("create cronjob");

    let job = trigger_cronjob(
        &env.resources,
        env.namespace(),
        "nightly",
        &WriteOptions::default(),
    )
    .await
    .expect("trigger");
    assert!(job.name().starts_with("nightly-manual-"), "{}", job.name());
    assert!(
        job.name().len() > "nightly-manual-".len(),
        "the server added a suffix"
    );
    assert_eq!(
        job.get_str("/metadata/annotations/cronjob.kubernetes.io~1instantiate"),
        Some("manual")
    );
    let owner = job.meta.controller_ref().expect("the cronjob owns the job");
    assert_eq!((&*owner.kind, &*owner.name), ("CronJob", "nightly"));
    assert_eq!(owner.uid, created.meta.uid.clone().expect("cronjob uid"));
    assert!(owner.block_owner_deletion);
    // The job runs what the template says.
    assert_eq!(
        job.to_value()["spec"]["template"]["spec"]["containers"][0]["image"],
        json!(images::PAUSE)
    );
    assert_eq!(
        job.to_value()["spec"]["template"]["spec"]["restartPolicy"],
        json!("Never")
    );

    // It is a real, readable object, and a second trigger makes another one.
    let live = env
        .resources
        .get(&job_gvk(), ns, job.name())
        .await
        .expect("get job");
    assert_eq!(live.meta.uid, job.meta.uid);
    let second = trigger_cronjob(
        &env.resources,
        env.namespace(),
        "nightly",
        &WriteOptions::default(),
    )
    .await
    .expect("second trigger");
    assert_ne!(second.name(), job.name());
}

#[tokio::test]
async fn a_dry_run_trigger_creates_nothing_and_a_missing_cronjob_is_not_found() {
    let Some(env) = setup().await else { return };
    let ns = Some(env.namespace());
    env.resources
        .create(
            &cronjob_gvk(),
            ns,
            &cronjob("nightly"),
            &WriteOptions::default(),
        )
        .await
        .expect("create cronjob");

    let preview = trigger_cronjob(
        &env.resources,
        env.namespace(),
        "nightly",
        &WriteOptions::dry_run(),
    )
    .await
    .expect("dry run");
    assert!(
        preview.name().starts_with("nightly-manual-"),
        "{}",
        preview.name()
    );
    let jobs = env
        .resources
        .list(&job_gvk(), ns, &oxikube_ports::ListOptions::default())
        .await
        .expect("list jobs");
    assert!(jobs.items.is_empty(), "a dry run stores nothing");

    let missing = trigger_cronjob(
        &env.resources,
        env.namespace(),
        "ghost",
        &WriteOptions::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound, "{missing}");
}

#[tokio::test]
async fn rollout_undo_returns_the_deployment_to_the_previous_template() {
    let Some(env) = setup().await else { return };
    let ns = Some(env.namespace());
    let options = WriteOptions::default();
    env.resources
        .create(&deployment_gvk(), ns, &deployment("web", 0), &options)
        .await
        .expect("create deployment");
    let history = |env: &Env| {
        let namespace = env.namespace().to_owned();
        let resources = env.resources.clone();
        async move { rollout_history(&resources, &namespace, "web").await }
    };
    wait_until("revision 1", common::DEADLINE, || async {
        let h = history(&env).await.ok()?;
        (h.len() == 1 && h[0].current).then_some(())
    })
    .await;

    // Revision 2: another image and a recorded cause.
    env.resources
        .patch(
            &deployment_gvk(),
            ns,
            "web",
            &Patch::strategic(json!({
                "metadata": {"annotations": {"kubernetes.io/change-cause": "bump pause"}},
                "spec": {"template": {"spec": {"containers": [{"name": "pause", "image": V2}]}}},
            })),
            &options,
        )
        .await
        .expect("roll out revision 2");
    let two = wait_until("revision 2", common::DEADLINE, || async {
        let h = history(&env).await.ok()?;
        (h.len() == 2 && h[1].current).then_some(h)
    })
    .await;
    assert_eq!(two[0].images, [V1]);
    assert_eq!(two[1].images, [V2]);
    assert_eq!(two[1].change_cause.as_deref(), Some("bump pause"));
    assert_eq!(two[0].change_cause, None);
    assert!(two[0].created.is_some() && two[0].created <= two[1].created);
    assert_eq!(deployed_image(&env, "web").await, V2);

    // A dry run shows the result and changes nothing.
    let preview = rollout_undo(
        &env.resources,
        env.namespace(),
        "web",
        None,
        &WriteOptions::dry_run(),
    )
    .await
    .expect("dry run undo");
    assert_eq!(
        (
            preview.from_revision,
            preview.to_revision,
            preview.unchanged
        ),
        (Some(2), 1, false)
    );
    let shown = preview.deployment.expect("the deployment as it would be");
    assert_eq!(
        shown.to_value()["spec"]["template"]["spec"]["containers"][0]["image"],
        json!(V1)
    );
    assert_eq!(deployed_image(&env, "web").await, V2);

    // Undo: the template is revision 1's again and the controller records a new revision.
    let undo = rollout_undo(&env.resources, env.namespace(), "web", None, &options)
        .await
        .expect("undo");
    assert_eq!(
        (undo.from_revision, undo.to_revision, undo.unchanged),
        (Some(2), 1, false)
    );
    assert_eq!(deployed_image(&env, "web").await, V1);
    let after = wait_until("revision 3", common::DEADLINE, || async {
        let h = history(&env).await.ok()?;
        h.iter().any(|r| r.current && r.revision == 3).then_some(h)
    })
    .await;
    // The old ReplicaSet was reused, so its revision moved up; the pod template has no stray
    // `pod-template-hash` from the copy.
    assert_eq!(after.iter().map(|r| r.revision).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(after[1].images, [V1]);
    let live = env
        .resources
        .get(&deployment_gvk(), ns, "web")
        .await
        .expect("get");
    assert!(
        live.to_value()["spec"]["template"]["metadata"]["labels"]["pod-template-hash"].is_null()
    );

    // Undoing to the template that already runs changes nothing.
    let same = rollout_undo(&env.resources, env.namespace(), "web", Some(3), &options)
        .await
        .expect("undo to current");
    assert!(same.unchanged && same.deployment.is_none());

    // A named revision back in time, and one that does not exist.
    let named = rollout_undo(&env.resources, env.namespace(), "web", Some(2), &options)
        .await
        .expect("undo to 2");
    assert_eq!(named.to_revision, 2);
    assert_eq!(deployed_image(&env, "web").await, V2);
    let unknown = rollout_undo(&env.resources, env.namespace(), "web", Some(99), &options)
        .await
        .unwrap_err();
    assert_eq!(unknown.kind(), ErrorKind::Validation, "{unknown}");
}

#[tokio::test]
async fn rollout_undo_of_a_paused_or_single_revision_deployment_is_refused() {
    let Some(env) = setup().await else { return };
    let ns = Some(env.namespace());
    let options = WriteOptions::default();
    env.resources
        .create(&deployment_gvk(), ns, &deployment("solo", 0), &options)
        .await
        .expect("create deployment");
    wait_until("revision 1", common::DEADLINE, || async {
        let h = rollout_history(&env.resources, env.namespace(), "solo")
            .await
            .ok()?;
        (h.len() == 1).then_some(())
    })
    .await;

    let none = rollout_undo(&env.resources, env.namespace(), "solo", None, &options)
        .await
        .unwrap_err();
    assert_eq!(none.kind(), ErrorKind::Validation, "{none}");

    env.resources
        .patch(
            &deployment_gvk(),
            ns,
            "solo",
            &Patch::merge(json!({"spec": {"paused": true}})),
            &options,
        )
        .await
        .expect("pause");
    let paused = rollout_undo(&env.resources, env.namespace(), "solo", Some(1), &options)
        .await
        .unwrap_err();
    assert_eq!(paused.kind(), ErrorKind::Conflict, "{paused}");

    let missing = rollout_history(&env.resources, env.namespace(), "ghost")
        .await
        .unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound, "{missing}");
}
