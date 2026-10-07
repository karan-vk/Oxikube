//! Kind integration for E04-S05: create, replace, patch kinds, server-side apply conflicts and
//! dry-run against a real API server. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`;
//! skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use kube::Api;
use kube::api::{DynamicObject, PostParams};
use kube::core::ApiResource;
use oxikube_domain::ErrorKind;
use oxikube_domain::{ConflictReason, Resource};
use oxikube_ports::{Patch, ResourceWriter, WriteOptions};
use oxikube_testkit::integration::TestNamespace;
use serde_json::json;

use common::mutations::{configmap, configmap_gvk, live, managers, setup};
use common::resources::adapter;
use common::{DEADLINE, TestCrd, wait_until};

fn apply(data: &[(&str, &str)], manager: &str, force: bool) -> Patch {
    let mut object = configmap("shared", &[], data);
    object["apiVersion"] = json!("v1");
    object["kind"] = json!("ConfigMap");
    Patch::apply(object, manager, force)
}

#[tokio::test]
async fn create_replace_and_the_three_patch_kinds_round_trip() {
    let Some((r, ns)) = setup().await else { return };
    let ns_name = ns.name();
    let ns = Some(ns_name);
    let write = WriteOptions::default();

    let created = r
        .create(
            &configmap_gvk(),
            ns,
            &configmap("cm", &[("l", "1")], &[("a", "1")]),
            &write,
        )
        .await
        .expect("create");
    assert_eq!(created.json["data"]["a"], "1");
    assert!(
        managers(&created).contains(&"oxikube".to_owned()),
        "{:?}",
        managers(&created)
    );

    let exists = r
        .create(&configmap_gvk(), ns, &configmap("cm", &[], &[]), &write)
        .await
        .unwrap_err();
    assert_eq!(exists.kind(), ErrorKind::Conflict);
    assert_eq!(
        exists.conflict_details().map(|d| d.reason),
        Some(ConflictReason::AlreadyExists)
    );

    let merged = r
        .patch(
            &configmap_gvk(),
            ns,
            "cm",
            &Patch::merge(json!({"data": {"b": "2"}})),
            &write,
        )
        .await
        .expect("merge patch");
    assert_eq!(merged.json["data"], json!({"a": "1", "b": "2"}));

    let strategic = r
        .patch(
            &configmap_gvk(),
            ns,
            "cm",
            &Patch::strategic(json!({"data": {"c": "3"}})),
            &write,
        )
        .await
        .expect("strategic patch");
    assert_eq!(strategic.json["data"]["c"], "3");

    let json_patched = r
        .patch(
            &configmap_gvk(),
            ns,
            "cm",
            &Patch::json(json!([
                {"op": "remove", "path": "/data/a"},
                {"op": "add", "path": "/data/d", "value": "4"},
            ])),
            &write,
        )
        .await
        .expect("json patch");
    assert_eq!(
        json_patched.json["data"],
        json!({"b": "2", "c": "3", "d": "4"})
    );

    let mut next = (*json_patched.json).clone();
    next["data"] = json!({"only": "this"});
    let replaced = r
        .replace(&configmap_gvk(), ns, "cm", &next, &write)
        .await
        .expect("replace");
    assert_eq!(replaced.json["data"], json!({"only": "this"}));
}

#[tokio::test]
async fn replace_with_a_stale_resource_version_is_a_stale_version_conflict() {
    let Some((r, ns)) = setup().await else { return };
    let ns_name = ns.name();
    let ns = Some(ns_name);
    let write = WriteOptions::default();
    let first = r
        .create(
            &configmap_gvk(),
            ns,
            &configmap("cm", &[], &[("a", "1")]),
            &write,
        )
        .await
        .expect("create");
    // Someone else changes the object, so `first`'s resourceVersion is stale.
    r.patch(
        &configmap_gvk(),
        ns,
        "cm",
        &Patch::merge(json!({"data": {"a": "2"}})),
        &write,
    )
    .await
    .expect("concurrent edit");

    let mut stale = (*first.json).clone();
    stale["data"]["a"] = json!("mine");
    let err = r
        .replace(&configmap_gvk(), ns, "cm", &stale, &write)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict, "{err}");
    let details = err.conflict_details().expect("conflict details");
    assert_eq!(details.reason, ConflictReason::StaleVersion);
    assert_eq!(
        live(&r, ns_name, "cm").await.unwrap().unwrap().json["data"]["a"],
        "2"
    );
}

#[tokio::test]
async fn server_side_apply_conflicts_name_the_other_manager_and_force_takes_over() {
    let Some((r, ns)) = setup().await else { return };
    let ns_name = ns.name();
    let ns = Some(ns_name);
    let write = WriteOptions::default();

    r.patch(
        &configmap_gvk(),
        ns,
        "shared",
        &apply(&[("k", "from-alpha")], "alpha", false),
        &write,
    )
    .await
    .expect("alpha applies");

    let err = r
        .patch(
            &configmap_gvk(),
            ns,
            "shared",
            &apply(&[("k", "from-oxikube")], "oxikube", false),
            &write,
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict, "{err}");
    let details = err.conflict_details().expect("conflict details");
    assert_eq!(details.reason, ConflictReason::FieldOwnership);
    assert_eq!(details.managers(), ["alpha"]);
    assert_eq!(details.causes[0].field, ".data.k");
    assert_eq!(
        live(&r, ns_name, "shared").await.unwrap().unwrap().json["data"]["k"],
        "from-alpha",
        "a refused apply changes nothing"
    );

    let forced = r
        .patch(
            &configmap_gvk(),
            ns,
            "shared",
            &apply(&[("k", "from-oxikube")], "oxikube", true),
            &write,
        )
        .await
        .expect("forced apply");
    assert_eq!(forced.json["data"]["k"], "from-oxikube");
    assert!(managers(&forced).contains(&"oxikube".to_owned()));
}

#[tokio::test]
async fn dry_run_returns_the_would_be_object_and_changes_nothing() {
    let Some((r, ns)) = setup().await else { return };
    let ns_name = ns.name();
    let ns = Some(ns_name);
    let dry = WriteOptions::dry_run();

    // A dry-run create returns the object (with a server-assigned uid) and stores nothing.
    let preview = r
        .create(
            &configmap_gvk(),
            ns,
            &configmap("ghost", &[], &[("a", "1")]),
            &dry,
        )
        .await
        .expect("dry-run create");
    assert_eq!(preview.json["data"]["a"], "1");
    assert!(preview.meta.uid.is_some());
    assert!(live(&r, ns_name, "ghost").await.unwrap().is_none());

    // A dry-run apply onto an existing object shows the merge result; the live object is untouched.
    let real = r
        .create(
            &configmap_gvk(),
            ns,
            &configmap("shared", &[], &[("a", "1")]),
            &WriteOptions::default(),
        )
        .await
        .expect("create");
    let applied = r
        .patch(
            &configmap_gvk(),
            ns,
            "shared",
            &apply(&[("b", "2")], "oxikube", false),
            &dry,
        )
        .await
        .expect("dry-run apply");
    assert_eq!(applied.json["data"]["b"], "2");
    let after = live(&r, ns_name, "shared").await.unwrap().unwrap();
    assert_eq!(after.json["data"], json!({"a": "1"}));
    assert_eq!(after.meta.resource_version, real.meta.resource_version);

    // A dry-run patch and replace likewise.
    let patched = r
        .patch(
            &configmap_gvk(),
            ns,
            "shared",
            &Patch::merge(json!({"data": {"c": "3"}})),
            &dry,
        )
        .await
        .expect("dry-run patch");
    assert_eq!(patched.json["data"]["c"], "3");
    let unchanged: Resource = live(&r, ns_name, "shared").await.unwrap().unwrap();
    assert_eq!(unchanged.meta.resource_version, real.meta.resource_version);
}

#[tokio::test]
async fn invalid_objects_come_back_as_validation_errors_with_field_paths() {
    let Some((r, ns)) = setup().await else { return };
    let err = r
        .create(
            &configmap_gvk(),
            Some(ns.name()),
            &configmap("Not_A_Valid_Name", &[], &[]),
            &WriteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation, "{err}");
    let details = err.validation_details().expect("validation details");
    assert!(
        details.causes.iter().any(|c| c.field == "metadata.name"),
        "{details:?}"
    );
}

#[tokio::test]
async fn custom_resources_accept_merge_and_apply_but_not_strategic_patches() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let crd = TestCrd::create(&client, kind.context.as_str()).await;
    let r = adapter(&client);

    let ar = ApiResource::from_gvk_with_plural(
        &kube::core::GroupVersionKind::gvk(&crd.gvk.group, "v1", "Gizmo"),
        "gizmos",
    );
    // The CRD is served a moment after it is created.
    let gizmos: Api<DynamicObject> = Api::namespaced_with((*client).clone(), ns.name(), &ar);
    let seed: DynamicObject = serde_json::from_value(json!({
        "apiVersion": crd.gvk.api_version(), "kind": "Gizmo",
        "metadata": {"name": "g1"}, "spec": {"size": 1},
    }))
    .expect("gizmo");
    wait_until("the Gizmo CRD to be served", DEADLINE, || async {
        gizmos.create(&PostParams::default(), &seed).await.ok()
    })
    .await;

    let write = WriteOptions::default();
    let gvk = crd.gvk.clone();
    // The adapter's registry learns the kind on a miss (2 s cooldown).
    let merged = wait_until(
        "the adapter to resolve the Gizmo kind",
        DEADLINE,
        || async {
            r.patch(
                &gvk,
                Some(ns.name()),
                "g1",
                &Patch::merge(json!({"spec": {"size": 2}})),
                &write,
            )
            .await
            .ok()
        },
    )
    .await;
    assert_eq!(merged.json["spec"]["size"], 2);

    let applied = r
        .patch(
            &gvk,
            Some(ns.name()),
            "g1",
            &Patch::apply(
                json!({"apiVersion": gvk.api_version(), "kind": "Gizmo",
                       "metadata": {"name": "g1"}, "spec": {"size": 3}}),
                "oxikube",
                true,
            ),
            &write,
        )
        .await
        .expect("apply to a custom resource");
    assert_eq!(applied.json["spec"]["size"], 3);

    let err = r
        .patch(
            &gvk,
            Some(ns.name()),
            "g1",
            &Patch::strategic(json!({"spec": {"size": 4}})),
            &write,
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported, "{err}");
}
