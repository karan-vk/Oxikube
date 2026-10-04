//! Kind integration for E04-S01: `resourceVersion` semantics, custom resources and RBAC errors
//! against a real server. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly
//! otherwise.
#![cfg(feature = "integration")]

mod common;

use k8s_openapi::api::rbac::v1::PolicyRule;
use kube::Api;
use kube::api::{DynamicObject, PostParams};
use kube::core::ApiResource;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ContextName, Gvk};
use oxikube_ports::{ListOptions, ResourceReader, VersionMatch};
use oxikube_testkit::integration::TestNamespace;
use serde_json::json;

use common::resources::{adapter, create_pods, pending_pod};
use common::{DEADLINE, TestCrd, TestServiceAccount, wait_until};

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn names(page: &oxikube_ports::ListPage) -> Vec<String> {
    let mut names: Vec<_> = page.items.iter().map(|r| r.name().to_owned()).collect();
    names.sort();
    names
}

#[tokio::test]
async fn resource_version_semantics_against_a_real_server() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let resources = adapter(&client);
    create_pods(
        &client,
        ns.name(),
        vec![pending_pod("rv-1", &[]), pending_pod("rv-2", &[])],
    )
    .await;

    let consistent = resources
        .list(&pod_gvk(), Some(ns.name()), &ListOptions::default())
        .await
        .expect("consistent read");
    assert_eq!(
        consistent.items.len(),
        2,
        "an unset version reads the latest"
    );
    let rv = consistent
        .resource_version
        .clone()
        .expect("list resource version");

    let mut any = ListOptions::default();
    any.resource_version = Some("0".into());
    resources
        .list(&pod_gvk(), Some(ns.name()), &any)
        .await
        .expect("any version");

    let not_older = ListOptions::default().at(rv.clone(), VersionMatch::NotOlderThan);
    let at_least = resources
        .list(&pod_gvk(), Some(ns.name()), &not_older)
        .await
        .expect("NotOlderThan");
    assert_eq!(at_least.items.len(), 2);

    let exact = ListOptions::default().at(rv.clone(), VersionMatch::Exact);
    let pinned = resources
        .list(&pod_gvk(), Some(ns.name()), &exact)
        .await
        .expect("Exact");
    assert_eq!(pinned.resource_version.as_deref(), Some(rv.as_str()));

    // A page's list resource version can start S02's watch; it is what the next list reports.
    let paged = resources
        .list(
            &pod_gvk(),
            Some(ns.name()),
            &ListOptions::default().limit(1),
        )
        .await
        .expect("first page");
    assert!(paged.has_more());
    assert!(paged.resource_version.is_some());
}

#[tokio::test]
async fn custom_resources_list_and_get_through_the_same_path() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let crd = TestCrd::create(&client, kind.context.as_str()).await;
    let resources = adapter(&client);

    // The CRD is served a moment after it is created; a miss re-discovers (2 s cooldown).
    let ar = ApiResource::from_gvk_with_plural(
        &kube::core::GroupVersionKind::gvk(&crd.gvk.group, "v1", "Gizmo"),
        "gizmos",
    );
    let gizmos: Api<DynamicObject> = Api::namespaced_with((*client).clone(), ns.name(), &ar);
    let make = |name: &str, size: i64| {
        serde_json::from_value::<DynamicObject>(json!({
            "apiVersion": crd.gvk.api_version(), "kind": "Gizmo",
            "metadata": {"name": name, "labels": {"shape": "round"}},
            "spec": {"size": size, "nested": {"list": [1, 2, 3]}},
        }))
        .expect("gizmo")
    };
    wait_until("the Gizmo CRD to be served", DEADLINE, || async {
        gizmos
            .create(&PostParams::default(), &make("g1", 1))
            .await
            .ok()
    })
    .await;
    gizmos
        .create(&PostParams::default(), &make("g2", 2))
        .await
        .expect("create g2");

    let page = wait_until(
        "the adapter to resolve the Gizmo kind",
        DEADLINE,
        || async {
            resources
                .list(&crd.gvk, Some(ns.name()), &ListOptions::default())
                .await
                .ok()
        },
    )
    .await;
    assert_eq!(names(&page), ["g1", "g2"]);
    let g2 = page.items.iter().find(|r| r.name() == "g2").unwrap();
    assert_eq!(g2.kind, crd.gvk);
    assert_eq!(g2.get_i64("/spec/size"), Some(2));
    assert_eq!(g2.get("/spec/nested/list"), Some(&json!([1, 2, 3])));

    let paged = resources
        .list_all(
            &crd.gvk,
            Some(ns.name()),
            &ListOptions::default().labels("shape=round").limit(1),
        )
        .await
        .expect("paged CRD list");
    assert_eq!(names(&paged), ["g1", "g2"].map(String::from).to_vec());
    let got = resources
        .get(&crd.gvk, Some(ns.name()), "g1")
        .await
        .expect("get");
    assert_eq!(got.get_i64("/spec/size"), Some(1));
}

#[tokio::test]
async fn a_restricted_account_gets_forbidden_not_a_crash() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    let pods_only = PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec!["pods".into()]),
        verbs: vec!["get".into(), "list".into()],
        ..PolicyRule::default()
    };
    let account =
        TestServiceAccount::create(&admin, ns.name(), "oxi-reader", vec![pods_only]).await;
    let context = ContextName::from("oxi-reader");
    let pool = kind.pool(kind.with_token_context(context.as_str(), &account.token));
    let client = pool.get(&context).await.expect("restricted client");
    let resources = adapter(&client);

    wait_until("the Role to allow listing pods", DEADLINE, || async {
        resources
            .list(&pod_gvk(), Some(ns.name()), &ListOptions::default())
            .await
            .ok()
    })
    .await;

    let secrets = Gvk::new("", "v1", "Secret");
    let err = resources
        .list(&secrets, Some(ns.name()), &ListOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(err.message().contains("cannot list resource"), "{err}");
    let err = resources
        .get(&secrets, Some(ns.name()), "any")
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}
