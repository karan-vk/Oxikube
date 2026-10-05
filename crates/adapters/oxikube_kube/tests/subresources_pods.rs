//! Kind integration for E04-S06 (pod subresources): eviction with and without a blocking
//! PodDisruptionBudget, ephemeral containers and in-place resize. Needs `cargo xtask kind-up`
//! and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use kube::Api;
use oxikube_domain::ErrorKind;
use oxikube_kube::{
    EphemeralContainerSpec, ResizeSpec, ephemeral_container_patch, eviction_blocked, resize_patch,
};
use oxikube_ports::{DeleteOptions, ResourceReader, ResourceWriter, Subresource, WriteOptions};
use serde_json::json;

use common::subresources::{Env, is_ready, live_pod, pod_gvk, running_pod, setup};
use common::{DEADLINE, wait_until};

/// Creates pod `name` and waits until it is Ready.
async fn ready_pod(env: &Env, name: &str, labels: &[(&str, &str)]) {
    let ns = Some(env.namespace());
    env.resources
        .create(
            &pod_gvk(),
            ns,
            &running_pod(name, labels),
            &WriteOptions::default(),
        )
        .await
        .expect("create pod");
    let last = std::cell::RefCell::new(String::new());
    wait_until("the pod to be Ready", DEADLINE, || async {
        let pod = live_pod(&env.resources, env.namespace(), name)
            .await
            .ok()
            .flatten()?;
        if is_ready(&pod) {
            return Some(());
        }
        // Say why on a slow cluster (a failed pull, an unschedulable pod), once per change.
        let status = pod.json["status"].to_string();
        if *last.borrow() != status {
            eprintln!("pod {name} not Ready yet: {status}");
            *last.borrow_mut() = status;
        }
        None
    })
    .await;
}

#[tokio::test]
async fn evicting_a_pod_without_a_budget_deletes_it() {
    let Some(env) = setup().await else { return };
    ready_pod(&env, "free", &[("app", "free")]).await;

    env.resources
        .evict(env.namespace(), "free", &DeleteOptions::default())
        .await
        .expect("evict");
    wait_until("the evicted pod to go away", DEADLINE, || async {
        let pod = live_pod(&env.resources, env.namespace(), "free")
            .await
            .ok()?;
        match pod {
            None => Some(()),
            Some(p) => p.json["metadata"]["deletionTimestamp"]
                .is_string()
                .then_some(()),
        }
    })
    .await;

    let gone = env
        .resources
        .evict(env.namespace(), "free", &DeleteOptions::default())
        .await;
    // Terminating or already gone: never a budget refusal.
    if let Err(err) = gone {
        assert!(eviction_blocked(&err).is_none(), "{err}");
    }
    let ghost = env
        .resources
        .evict(env.namespace(), "ghost", &DeleteOptions::default())
        .await
        .unwrap_err();
    assert_eq!(ghost.kind(), ErrorKind::NotFound, "{ghost}");
}

#[tokio::test]
async fn a_blocking_budget_refuses_the_eviction_as_a_retryable_error() {
    let Some(env) = setup().await else { return };
    let ns = env.namespace();
    ready_pod(&env, "guarded", &[("app", "guarded")]).await;

    let pdbs = Api::<PodDisruptionBudget>::namespaced((*env.client).clone(), ns);
    let pdb: PodDisruptionBudget = serde_json::from_value(json!({
        "apiVersion": "policy/v1", "kind": "PodDisruptionBudget",
        "metadata": {"name": "keep-one"},
        "spec": {"minAvailable": 1, "selector": {"matchLabels": {"app": "guarded"}}},
    }))
    .expect("pdb");
    pdbs.create(&kube::api::PostParams::default(), &pdb)
        .await
        .expect("create pdb");
    wait_until("the budget to count the healthy pod", DEADLINE, || async {
        let status = pdbs.get("keep-one").await.ok()?.status?;
        (status.current_healthy == Some(1)).then_some(())
    })
    .await;

    let err = env
        .resources
        .evict(ns, "guarded", &DeleteOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Network, "{err}");
    assert!(err.is_retryable());
    let blocked = eviction_blocked(&err).expect("a budget refusal carries the marker");
    assert!(
        blocked.reason.contains("keep-one"),
        "the reason names the budget: {}",
        blocked.reason
    );
    let still = live_pod(&env.resources, ns, "guarded").await.expect("get");
    assert!(
        still.is_some_and(|p| p.json["metadata"]["deletionTimestamp"].is_null()),
        "a refused eviction leaves the pod alone"
    );

    // Dry run is refused by the same budget.
    let dry = env
        .resources
        .evict(ns, "guarded", &DeleteOptions::dry_run())
        .await
        .unwrap_err();
    assert!(eviction_blocked(&dry).is_some(), "{dry}");

    // Drop the budget and the eviction goes through.
    pdbs.delete("keep-one", &kube::api::DeleteParams::default())
        .await
        .expect("delete pdb");
    wait_until("the eviction to be allowed", DEADLINE, || async {
        env.resources
            .evict(ns, "guarded", &DeleteOptions::default())
            .await
            .ok()
    })
    .await;
}

#[tokio::test]
async fn an_ephemeral_container_is_added_through_its_subresource() {
    let Some(env) = setup().await else { return };
    let ns = Some(env.namespace());
    ready_pod(&env, "debuggee", &[("app", "debuggee")]).await;

    let patch = ephemeral_container_patch(&EphemeralContainerSpec {
        name: "dbg".into(),
        image: "registry.k8s.io/e2e-test-images/busybox:1.36.1-1".into(),
        command: vec!["sleep".into(), "3600".into()],
        target_container: Some("pause".into()),
        ..EphemeralContainerSpec::default()
    });
    let pod = env
        .resources
        .patch_subresource(
            &pod_gvk(),
            ns,
            "debuggee",
            &Subresource::EphemeralContainers,
            &patch,
            &WriteOptions::default(),
        )
        .await
        .expect("patch ephemeralcontainers");
    assert_eq!(pod["spec"]["ephemeralContainers"][0]["name"], "dbg");

    let read = env
        .resources
        .get_subresource(
            &pod_gvk(),
            ns,
            "debuggee",
            &Subresource::EphemeralContainers,
        )
        .await
        .expect("get ephemeralcontainers");
    assert_eq!(read["spec"]["ephemeralContainers"][0]["name"], "dbg");
    assert_eq!(
        read["spec"]["ephemeralContainers"][0]["targetContainerName"],
        "pause"
    );

    // The main object cannot take an ephemeral container: only the subresource can.
    let direct = env
        .resources
        .patch(
            &pod_gvk(),
            ns,
            "debuggee",
            &ephemeral_container_patch(&EphemeralContainerSpec {
                name: "second".into(),
                image: "registry.k8s.io/e2e-test-images/busybox:1.36.1-1".into(),
                ..EphemeralContainerSpec::default()
            }),
            &WriteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(direct.kind(), ErrorKind::Validation, "{direct}");
}

#[tokio::test]
async fn a_pod_is_resized_in_place_where_the_server_supports_it() {
    let Some(env) = setup().await else { return };
    let ns = Some(env.namespace());
    ready_pod(&env, "elastic", &[("app", "elastic")]).await;

    let patch = resize_patch(&ResizeSpec {
        container: "pause".into(),
        requests: vec![("cpu".into(), "20m".into())],
        limits: vec![("cpu".into(), "100m".into())],
    });
    let resized = match env
        .resources
        .patch_subresource(
            &pod_gvk(),
            ns,
            "elastic",
            &Subresource::Resize,
            &patch,
            &WriteOptions::default(),
        )
        .await
    {
        Ok(pod) => pod,
        Err(err) if err.kind() == ErrorKind::Unsupported => {
            eprintln!("skipping: this cluster does not serve pods/resize ({err})");
            return;
        }
        Err(err) => panic!("resize failed: {err}"),
    };
    let resources = &resized["spec"]["containers"][0]["resources"];
    assert_eq!(resources["requests"]["cpu"], "20m");
    assert_eq!(resources["limits"]["cpu"], "100m");
    // Untouched resources keep their values.
    assert_eq!(resources["requests"]["memory"], "8Mi");

    let read = env
        .resources
        .get_subresource(&pod_gvk(), ns, "elastic", &Subresource::Resize)
        .await
        .expect("get resize");
    assert_eq!(
        read["spec"]["containers"][0]["resources"]["requests"]["cpu"],
        "20m"
    );
}
