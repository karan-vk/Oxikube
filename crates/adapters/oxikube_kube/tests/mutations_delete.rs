//! Kind integration for E04-S05: delete with each propagation policy and delete-collection by
//! label. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use oxikube_domain::ErrorKind;
use oxikube_ports::{
    DeleteCollectionOutcome, DeleteOptions, DeleteOutcome, ListOptions, PropagationPolicy,
    ResourceWriter, WriteOptions,
};
use oxikube_testkit::integration::TestNamespace;

use common::mutations::{configmap, configmap_gvk, live, owned_configmap};
use common::resources::adapter;
use common::{DEADLINE, wait_until};

async fn setup() -> Option<(oxikube_kube::KubeResources, TestNamespace)> {
    let kind = common::kind().await?;
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    Some((adapter(&client), ns))
}

/// A parent ConfigMap and a child that names it as owner.
async fn family(r: &oxikube_kube::KubeResources, ns: &str) {
    let write = WriteOptions::default();
    let parent = r
        .create(
            &configmap_gvk(),
            Some(ns),
            &configmap("parent", &[], &[]),
            &write,
        )
        .await
        .expect("create parent");
    r.create(
        &configmap_gvk(),
        Some(ns),
        &owned_configmap("child", &parent),
        &write,
    )
    .await
    .expect("create child");
}

async fn gone(r: &oxikube_kube::KubeResources, ns: &str, name: &str) {
    wait_until(&format!("{name} to be gone"), DEADLINE, || async {
        live(r, ns, name)
            .await
            .expect("get")
            .is_none()
            .then_some(())
    })
    .await;
}

#[tokio::test]
async fn a_plain_delete_reports_deleted_and_a_second_one_not_found() {
    let Some((r, ns)) = setup().await else { return };
    let ns = ns.name();
    r.create(
        &configmap_gvk(),
        Some(ns),
        &configmap("one", &[], &[]),
        &WriteOptions::default(),
    )
    .await
    .expect("create");
    let outcome = r
        .delete(&configmap_gvk(), Some(ns), "one", &DeleteOptions::default())
        .await
        .expect("delete");
    assert_eq!(outcome, DeleteOutcome::Deleted);
    let err = r
        .delete(&configmap_gvk(), Some(ns), "one", &DeleteOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn a_dry_run_delete_leaves_the_object() {
    let Some((r, ns)) = setup().await else { return };
    let ns = ns.name();
    r.create(
        &configmap_gvk(),
        Some(ns),
        &configmap("keep", &[], &[]),
        &WriteOptions::default(),
    )
    .await
    .expect("create");
    r.delete(
        &configmap_gvk(),
        Some(ns),
        "keep",
        &DeleteOptions::dry_run(),
    )
    .await
    .expect("dry-run delete");
    assert!(live(&r, ns, "keep").await.unwrap().is_some());
}

#[tokio::test]
async fn background_propagation_deletes_the_dependents_too() {
    let Some((r, ns)) = setup().await else { return };
    let ns = ns.name();
    family(&r, ns).await;
    let options = DeleteOptions::default().propagation(PropagationPolicy::Background);
    r.delete(&configmap_gvk(), Some(ns), "parent", &options)
        .await
        .expect("delete");
    gone(&r, ns, "parent").await;
    gone(&r, ns, "child").await;
}

#[tokio::test]
async fn orphan_propagation_keeps_the_dependents_and_drops_their_owner_reference() {
    let Some((r, ns)) = setup().await else { return };
    let ns = ns.name();
    family(&r, ns).await;
    let options = DeleteOptions::default().propagation(PropagationPolicy::Orphan);
    r.delete(&configmap_gvk(), Some(ns), "parent", &options)
        .await
        .expect("delete");
    gone(&r, ns, "parent").await;
    // The garbage collector strips the owner reference asynchronously.
    wait_until("the child to be orphaned", DEADLINE, || async {
        let child = live(&r, ns, "child").await.expect("get")?;
        child.meta.owner_refs.is_empty().then_some(())
    })
    .await;
}

#[tokio::test]
async fn foreground_propagation_holds_the_owner_until_the_dependents_are_gone() {
    let Some((r, ns)) = setup().await else { return };
    let ns = ns.name();
    family(&r, ns).await;
    let options = DeleteOptions::default().propagation(PropagationPolicy::Foreground);
    let outcome = r
        .delete(&configmap_gvk(), Some(ns), "parent", &options)
        .await
        .expect("delete");
    // The owner carries the foregroundDeletion finalizer while its dependents are deleted.
    if let DeleteOutcome::Deleting(object) = &outcome {
        assert!(object.meta.deletion.is_some(), "{object:?}");
    }
    gone(&r, ns, "child").await;
    gone(&r, ns, "parent").await;
}

#[tokio::test]
async fn delete_collection_removes_only_what_the_selector_matches() {
    let Some((r, ns)) = setup().await else { return };
    let ns = ns.name();
    let write = WriteOptions::default();
    for (name, group) in [
        ("a", "doomed"),
        ("b", "doomed"),
        ("c", "doomed"),
        ("d", "safe"),
    ] {
        r.create(
            &configmap_gvk(),
            Some(ns),
            &configmap(name, &[("group", group)], &[]),
            &write,
        )
        .await
        .expect("create");
    }

    let dry = r
        .delete_collection(
            &configmap_gvk(),
            Some(ns),
            &ListOptions::default().labels("group=doomed"),
            &DeleteOptions::dry_run(),
        )
        .await
        .expect("dry-run delete collection");
    if let DeleteCollectionOutcome::Deleting(items) = dry {
        assert_eq!(items.len(), 3);
    }
    assert!(
        live(&r, ns, "a").await.unwrap().is_some(),
        "a dry run deletes nothing"
    );

    r.delete_collection(
        &configmap_gvk(),
        Some(ns),
        &ListOptions::default().labels("group=doomed"),
        &DeleteOptions::default(),
    )
    .await
    .expect("delete collection");
    for name in ["a", "b", "c"] {
        gone(&r, ns, name).await;
    }
    assert!(live(&r, ns, "d").await.unwrap().is_some());
}
