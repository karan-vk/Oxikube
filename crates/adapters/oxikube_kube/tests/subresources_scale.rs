//! Kind integration for E04-S06 (scale and status): the scale subresource of a Deployment
//! and of a CRD, `status` on a CRD that has the subresource, and the errors for kinds that do
//! not. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use kube::Api;
use kube::api::{DynamicObject, PostParams};
use kube::core::ApiResource;
use oxikube_domain::ErrorKind;
use oxikube_ports::{Patch, ResourceReader, ResourceWriter, Subresource, WriteOptions};
use serde_json::json;

use common::mutations::{configmap, configmap_gvk};
use common::subresources::{Env, SubCrd, deployment, deployment_gvk, setup};
use common::{DEADLINE, wait_until};

#[tokio::test]
async fn scaling_a_deployment_goes_through_the_scale_subresource_and_reads_back() {
    let Some(env) = setup().await else { return };
    let (r, ns) = (&env.resources, Some(env.namespace()));
    let write = WriteOptions::default();
    r.create(&deployment_gvk(), ns, &deployment("web", 0), &write)
        .await
        .expect("create deployment");

    let before = r
        .get_scale(&deployment_gvk(), ns, "web")
        .await
        .expect("get_scale");
    assert_eq!(before.replicas, 0);
    assert_eq!(before.selector.as_deref(), Some("app=web"));

    let scaled = r
        .scale(&deployment_gvk(), ns, "web", 2, &write)
        .await
        .expect("scale to 2");
    assert_eq!(scaled.replicas, 2);
    assert!(scaled.resource_version.is_some());

    // Read back through the subresource and through the object.
    let again = r
        .get_scale(&deployment_gvk(), ns, "web")
        .await
        .expect("get_scale");
    assert_eq!(again.replicas, 2);
    assert_eq!(again.selector.as_deref(), Some("app=web"));
    let object = r.get(&deployment_gvk(), ns, "web").await.expect("get");
    assert_eq!(object.to_value()["spec"]["replicas"], 2);

    // A dry run answers with the would-be scale and changes nothing.
    let dry = r
        .scale(&deployment_gvk(), ns, "web", 5, &WriteOptions::dry_run())
        .await
        .expect("dry-run scale");
    assert_eq!(dry.replicas, 5);
    let live = r
        .get_scale(&deployment_gvk(), ns, "web")
        .await
        .expect("get_scale");
    assert_eq!(live.replicas, 2);

    r.scale(&deployment_gvk(), ns, "web", 0, &write)
        .await
        .expect("scale to zero");
    let object = r.get(&deployment_gvk(), ns, "web").await.expect("get");
    assert_eq!(object.to_value()["spec"]["replicas"], 0);
}

#[tokio::test]
async fn scale_errors_are_classified() {
    let Some(env) = setup().await else { return };
    let (r, ns) = (&env.resources, Some(env.namespace()));
    let write = WriteOptions::default();

    let missing = r
        .scale(&deployment_gvk(), ns, "ghost", 1, &write)
        .await
        .unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound, "{missing}");

    let negative = r
        .scale(&deployment_gvk(), ns, "web", -1, &write)
        .await
        .unwrap_err();
    assert_eq!(negative.kind(), ErrorKind::Validation);

    // A ConfigMap has no scale subresource.
    r.create(&configmap_gvk(), ns, &configmap("cm", &[], &[]), &write)
        .await
        .expect("create configmap");
    let none = r.get_scale(&configmap_gvk(), ns, "cm").await.unwrap_err();
    assert_eq!(none.kind(), ErrorKind::Unsupported, "{none}");
}

/// Creates a `Widget` once the CRD is served; the adapter's registry learns the kind on a
/// miss (2 s cooldown), so the first adapter call is retried.
async fn seed_widget(env: &Env, crd: &SubCrd, name: &str) {
    let ar = ApiResource::from_gvk_with_plural(
        &kube::core::GroupVersionKind::gvk(&crd.gvk.group, "v1", "Widget"),
        "widgets",
    );
    let api: Api<DynamicObject> = Api::namespaced_with((*env.client).clone(), env.namespace(), &ar);
    let seed: DynamicObject = serde_json::from_value(json!({
        "apiVersion": crd.gvk.api_version(), "kind": "Widget",
        "metadata": {"name": name}, "spec": {"size": 1},
    }))
    .expect("widget");
    wait_until("the Widget CRD to be served", DEADLINE, || async {
        api.create(&PostParams::default(), &seed).await.ok()
    })
    .await;
}

#[tokio::test]
async fn a_crd_with_scale_and_status_subresources_serves_both() {
    let Some(env) = setup().await else { return };
    let crd = SubCrd::create(&env.client, &env.context).await;
    seed_widget(&env, &crd, "w1").await;
    let (r, ns, gvk) = (&env.resources, Some(env.namespace()), &crd.gvk);
    let write = WriteOptions::default();

    let scale = wait_until(
        "the adapter to resolve the Widget kind",
        DEADLINE,
        || async { r.get_scale(gvk, ns, "w1").await.ok() },
    )
    .await;
    assert_eq!(scale.replicas, 1);

    let scaled = r
        .scale(gvk, ns, "w1", 4, &write)
        .await
        .expect("scale a CRD");
    assert_eq!(scaled.replicas, 4);
    assert_eq!(
        r.get(gvk, ns, "w1").await.expect("get").to_value()["spec"]["size"],
        4
    );

    // `status` is a separate write path: a main-resource patch cannot set it, the subresource can.
    let status = r
        .patch_subresource(
            gvk,
            ns,
            "w1",
            &Subresource::Status,
            &Patch::merge(json!({"status": {"phase": "Ready", "size": 4}})),
            &write,
        )
        .await
        .expect("patch status");
    assert_eq!(status["status"]["phase"], "Ready");
    let read = r
        .get_subresource(gvk, ns, "w1", &Subresource::Status)
        .await
        .expect("get status");
    assert_eq!(read["status"], json!({"phase": "Ready", "size": 4}));
    assert_eq!(
        r.get_scale(gvk, ns, "w1")
            .await
            .expect("get_scale")
            .current_replicas,
        4,
        "status.size feeds the scale's observed replicas"
    );

    let mut replaced = read.clone();
    replaced["status"]["phase"] = json!("Done");
    let after = r
        .replace_subresource(gvk, ns, "w1", &Subresource::Status, &replaced, &write)
        .await
        .expect("replace status");
    assert_eq!(after["status"]["phase"], "Done");

    // Spec writes through the status subresource are ignored by the server.
    let ignored = r
        .patch_subresource(
            gvk,
            ns,
            "w1",
            &Subresource::Status,
            &Patch::merge(json!({"spec": {"size": 99}})),
            &write,
        )
        .await
        .expect("patch spec via status");
    assert_eq!(ignored["spec"]["size"], 4);
}

#[tokio::test]
async fn a_crd_without_subresources_answers_unsupported() {
    let Some(env) = setup().await else { return };
    let crd = SubCrd::create_plain(&env.client, &env.context).await;
    seed_widget(&env, &crd, "w1").await;
    let (r, ns, gvk) = (&env.resources, Some(env.namespace()), &crd.gvk);

    let err = wait_until(
        "the adapter to resolve the Widget kind",
        DEADLINE,
        || async {
            let got = r.get_scale(gvk, ns, "w1").await;
            eprintln!("DEBUG {got:?}");
            match got {
                Err(e) if e.kind() == ErrorKind::Unsupported && e.message().contains("scale") => {
                    Some(e)
                }
                _ => None,
            }
        },
    )
    .await;
    assert!(err.message().contains("subresource"), "{err}");

    let status = r
        .get_subresource(gvk, ns, "w1", &Subresource::Status)
        .await
        .unwrap_err();
    assert_eq!(status.kind(), ErrorKind::Unsupported, "{status}");
}
