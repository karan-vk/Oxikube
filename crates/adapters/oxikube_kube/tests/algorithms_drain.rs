//! Kind integration for E04-S07 (drain): cordon, eviction against a PodDisruptionBudget that
//! first blocks and then allows, and the filters. Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
//!
//! The real node is never touched. Each test makes a Node object no kubelet backs (a unique
//! `oxi-fake-<rand>`, tainted so the scheduler never uses it, deleted on drop) and binds its
//! pods to it with `spec.nodeName`. Nothing runs them, so a pod is made `Running` and `Ready` by
//! writing its status, which is what a budget needs to count it healthy, and evictions use
//! `gracePeriodSeconds: 0` because no kubelet would confirm a graceful delete.
#![cfg(feature = "integration")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;
use oxikube_kube::{DrainOptions, DrainProgress, SkipReason, drain, drain_to_completion};
use oxikube_ports::{
    Patch, ResourcePort, ResourceReader, ResourceWriter, Subresource, WriteOptions,
};
use serde_json::{Value, json};

use common::subresources::{Env, FakeNode, PAUSE, live_pod, pod_gvk, setup};
use common::wait_until;

fn node_gvk() -> Gvk {
    Gvk::new("", "v1", "Node")
}

fn pdb_gvk() -> Gvk {
    Gvk::new("policy", "v1", "PodDisruptionBudget")
}

/// A pod bound to `node`, labelled `app=web`, with `extra` merged into its `spec`.
fn bound_pod(name: &str, node: &str, extra: Value) -> Value {
    let mut spec = json!({
        "nodeName": node,
        "terminationGracePeriodSeconds": 0,
        "containers": [{"name": "pause", "image": PAUSE}],
    });
    for (key, value) in extra.as_object().into_iter().flatten() {
        spec[key] = value.clone();
    }
    json!({"metadata": {"name": name, "labels": {"app": "web"}}, "spec": spec})
}

/// Creates the pod and writes a Running, Ready status for it.
async fn ready_pod(env: &Env, pod: Value) {
    let name = pod["metadata"]["name"].as_str().expect("name").to_owned();
    let ns = Some(env.namespace());
    env.resources
        .create(&pod_gvk(), ns, &pod, &WriteOptions::default())
        .await
        .expect("create pod");
    env.resources
        .patch_subresource(
            &pod_gvk(),
            ns,
            &name,
            &Subresource::Status,
            &Patch::merge(json!({"status": {
                "phase": "Running",
                "conditions": [{"type": "Ready", "status": "True"}],
            }})),
            &WriteOptions::default(),
        )
        .await
        .expect("make the pod Ready");
}

async fn unschedulable(env: &Env, node: &FakeNode) -> bool {
    let live = env
        .resources
        .get(&node_gvk(), None, &node.name)
        .await
        .expect("get node");
    live.get_bool("/spec/unschedulable") == Some(true)
}

fn port(env: &Env) -> Arc<dyn ResourcePort> {
    Arc::new(env.resources.clone())
}

/// Fast retries, no grace: the pods have no kubelet to confirm a graceful delete.
fn quick() -> DrainOptions {
    DrainOptions {
        grace_period_secs: Some(0),
        retry_initial: Duration::from_millis(200),
        retry_max: Duration::from_millis(500),
        poll_interval: Duration::from_millis(100),
        timeout: Duration::from_secs(60),
        force: true,
        ..DrainOptions::default()
    }
}

#[tokio::test]
async fn a_drain_waits_out_a_budget_that_blocks_then_allows_and_evicts_everything() {
    let Some(env) = setup().await else { return };
    let node = FakeNode::create(&env.client, &env.context).await;
    let ns = Some(env.namespace());
    for name in ["web-1", "web-2"] {
        ready_pod(&env, bound_pod(name, &node.name, json!({}))).await;
    }
    // A pod with scratch space (evicted because the flag says so), and a mirror pod (skipped).
    let mut scratch = bound_pod(
        "scratch",
        &node.name,
        json!({"volumes": [{"name": "tmp", "emptyDir": {}}]}),
    );
    scratch["metadata"]["labels"] = json!({"app": "scratch"});
    ready_pod(&env, scratch).await;
    let mut mirror = bound_pod("static", &node.name, json!({}));
    mirror["metadata"]["annotations"] = json!({"kubernetes.io/config.mirror": "abc"});
    mirror["metadata"]["labels"] = json!({"app": "static"});
    ready_pod(&env, mirror).await;

    // The budget wants both web pods: no disruption is allowed yet.
    env.resources
        .create(
            &pdb_gvk(),
            ns,
            &json!({
                "metadata": {"name": "keep-web"},
                "spec": {"minAvailable": 2, "selector": {"matchLabels": {"app": "web"}}},
            }),
            &WriteOptions::default(),
        )
        .await
        .expect("create pdb");
    wait_until(
        "the budget to count both pods",
        common::DEADLINE,
        || async {
            let pdb = env.resources.get(&pdb_gvk(), ns, "keep-web").await.ok()?;
            (pdb.get_i64("/status/currentHealthy") == Some(2)
                && pdb.get_i64("/status/disruptionsAllowed") == Some(0))
            .then_some(())
        },
    )
    .await;

    let options = DrainOptions {
        ignore_daemonsets: true,
        delete_emptydir_data: true,
        ..quick()
    };
    let mut stream = Box::pin(drain(port(&env), node.name.clone(), options));
    let mut seen: Vec<DrainProgress> = Vec::new();
    let mut relaxed = false;
    while let Some(step) = stream.next().await {
        let step = step.expect("the drain does not fail as a whole");
        // The first refusal proves the budget blocks; now let it allow, as an operator would.
        if matches!(step, DrainProgress::Blocked { .. }) && !relaxed {
            relaxed = true;
            env.resources
                .patch(
                    &pdb_gvk(),
                    ns,
                    "keep-web",
                    &Patch::merge(json!({"spec": {"minAvailable": 0}})),
                    &WriteOptions::default(),
                )
                .await
                .expect("relax the budget");
        }
        seen.push(step);
    }

    assert!(relaxed, "the budget never blocked: {seen:?}");
    let blocked_reason = seen.iter().find_map(|s| match s {
        DrainProgress::Blocked { reason, .. } => Some(reason.clone()),
        _ => None,
    });
    assert!(
        blocked_reason.is_some_and(|r| r.contains("keep-web")),
        "the budget's explanation names it"
    );
    let Some(DrainProgress::Finished(summary)) = seen.last() else {
        panic!("not finished: {seen:?}");
    };
    assert!(summary.is_complete(), "{summary:?}");
    let mut evicted: Vec<_> = summary.evicted.iter().map(|p| p.name.as_str()).collect();
    evicted.sort_unstable();
    assert_eq!(evicted, ["scratch", "web-1", "web-2"]);
    assert_eq!(summary.skipped.len(), 1);
    assert_eq!(summary.skipped[0].pod.name, "static");
    assert_eq!(summary.skipped[0].reason, SkipReason::Mirror);

    // The first two steps are the plan and the cordon; the node is cordoned and its pods gone.
    assert!(matches!(seen[0], DrainProgress::Planned { .. }));
    assert!(matches!(
        seen[1],
        DrainProgress::Cordoned { already: false }
    ));
    assert!(unschedulable(&env, &node).await);
    for name in ["web-1", "web-2", "scratch"] {
        let pod = live_pod(&env.resources, env.namespace(), name)
            .await
            .expect("get");
        assert!(pod.is_none(), "{name} is gone");
    }
    assert!(
        live_pod(&env.resources, env.namespace(), "static")
            .await
            .expect("get")
            .is_some(),
        "the mirror pod stays"
    );
}

#[tokio::test]
async fn a_blocking_pod_refuses_the_drain_before_the_node_is_cordoned() {
    let Some(env) = setup().await else { return };
    let node = FakeNode::create(&env.client, &env.context).await;
    // No controller owns this pod and `force` is not set.
    ready_pod(&env, bound_pod("lone", &node.name, json!({}))).await;

    let options = DrainOptions {
        force: false,
        ..quick()
    };
    let err = drain_to_completion(drain(port(&env), node.name.clone(), options))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation, "{err}");
    assert!(err.to_string().contains("lone"), "{err}");
    assert!(!unschedulable(&env, &node).await, "nothing was cordoned");
    assert!(
        live_pod(&env.resources, env.namespace(), "lone")
            .await
            .expect("get")
            .is_some()
    );
}

#[tokio::test]
async fn a_dry_run_lists_what_would_be_evicted_and_changes_nothing() {
    let Some(env) = setup().await else { return };
    let node = FakeNode::create(&env.client, &env.context).await;
    ready_pod(&env, bound_pod("web-1", &node.name, json!({}))).await;

    let options = DrainOptions {
        dry_run: true,
        ..quick()
    };
    let summary = drain_to_completion(drain(port(&env), node.name.clone(), options))
        .await
        .expect("dry run");
    assert!(summary.dry_run);
    assert_eq!(summary.evicted.len(), 1);
    assert_eq!(summary.evicted[0].name, "web-1");
    assert!(!unschedulable(&env, &node).await);
    assert!(
        live_pod(&env.resources, env.namespace(), "web-1")
            .await
            .expect("get")
            .is_some()
    );
}

#[tokio::test]
async fn a_budget_that_stays_closed_fails_the_pod_at_the_timeout() {
    let Some(env) = setup().await else { return };
    let node = FakeNode::create(&env.client, &env.context).await;
    let ns = Some(env.namespace());
    ready_pod(&env, bound_pod("web-1", &node.name, json!({}))).await;
    env.resources
        .create(
            &pdb_gvk(),
            ns,
            &json!({
                "metadata": {"name": "keep-web"},
                "spec": {"minAvailable": 1, "selector": {"matchLabels": {"app": "web"}}},
            }),
            &WriteOptions::default(),
        )
        .await
        .expect("create pdb");
    wait_until("the budget to count the pod", common::DEADLINE, || async {
        let pdb = env.resources.get(&pdb_gvk(), ns, "keep-web").await.ok()?;
        (pdb.get_i64("/status/currentHealthy") == Some(1)).then_some(())
    })
    .await;

    let options = DrainOptions {
        timeout: Duration::from_secs(2),
        ..quick()
    };
    let steps: Vec<_> = drain(port(&env), node.name.clone(), options)
        .collect()
        .await;
    let steps: Vec<DrainProgress> = steps.into_iter().map(|s| s.expect("step")).collect();
    assert!(
        steps
            .iter()
            .any(|s| matches!(s, DrainProgress::Blocked { .. }))
    );
    let Some(DrainProgress::Finished(summary)) = steps.last() else {
        panic!("not finished");
    };
    assert_eq!(summary.failed.len(), 1);
    assert!(
        live_pod(&env.resources, env.namespace(), "web-1")
            .await
            .expect("get")
            .is_some_and(|p| p.meta.deletion.is_none()),
        "a refused eviction leaves the pod alone"
    );
}
