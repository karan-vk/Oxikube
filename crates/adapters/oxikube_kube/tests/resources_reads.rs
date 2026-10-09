//! Kind integration for E04-S01: label and field selectors, typed vs dynamic golden compare and
//! get / get_opt. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;
use oxikube_kube::{AccessPath, ManagedFields, ResourcesConfig};
use oxikube_ports::{ListOptions, ResourceReader};
use oxikube_testkit::integration::TestNamespace;

use common::resources::{
    adapter, adapter_with, bound_pod, create_pods, first_difference, pending_pod, without_nulls,
};
use common::{DEADLINE, wait_until};

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn names(page: &oxikube_ports::ListPage) -> Vec<String> {
    let mut names: Vec<_> = page.items.iter().map(|r| r.name().to_owned()).collect();
    names.sort();
    names
}

/// Typed and dynamic reads of one stored object agree on everything but explicit `null`s
/// (see `without_nulls`): same metadata, same kind, same JSON.
fn assert_same(typed: &oxikube_domain::Resource, dynamic: &oxikube_domain::Resource, name: &str) {
    let (t, d) = (
        without_nulls(&typed.to_value()),
        without_nulls(&dynamic.to_value()),
    );
    assert_eq!(
        first_difference(&t, &d),
        None,
        "typed vs dynamic read of {name}"
    );
    assert_eq!(typed.kind, dynamic.kind);
    assert_eq!(typed.meta, dynamic.meta);
}

/// The kind cluster's single node, found through the adapter itself.
async fn node_name(resources: &impl ResourceReader) -> String {
    let nodes = resources
        .list(&Gvk::new("", "v1", "Node"), None, &ListOptions::default())
        .await
        .expect("list nodes");
    nodes.items[0].name().to_owned()
}

#[tokio::test]
async fn label_and_field_selectors_return_the_expected_subsets() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let resources = adapter(&client);
    let node = node_name(&resources).await;

    create_pods(
        &client,
        ns.name(),
        vec![
            pending_pod("a-1", &[("tier", "a")]),
            pending_pod("a-2", &[("tier", "a")]),
            pending_pod("b-1", &[("tier", "b")]),
            bound_pod("bound-a", &[("tier", "a")], &node),
            bound_pod("bound-b", &[("tier", "b")], &node),
        ],
    )
    .await;
    let list = |options: ListOptions| {
        let resources = resources.clone();
        let namespace = ns.name().to_owned();
        async move {
            resources
                .list(&pod_gvk(), Some(&namespace), &options)
                .await
                .expect("list")
        }
    };

    assert_eq!(names(&list(ListOptions::default()).await).len(), 5);
    assert_eq!(
        names(&list(ListOptions::default().labels("tier=a")).await),
        ["a-1", "a-2", "bound-a"]
    );
    assert_eq!(
        names(&list(ListOptions::default().labels("tier in (a,b),tier!=b")).await),
        ["a-1", "a-2", "bound-a"]
    );
    assert_eq!(
        names(&list(ListOptions::default().fields(format!("spec.nodeName={node}"))).await),
        ["bound-a", "bound-b"]
    );
    assert_eq!(
        names(&list(ListOptions::default().fields("spec.nodeName=")).await),
        ["a-1", "a-2", "b-1"],
        "an empty node name selects the unscheduled pods"
    );
    assert_eq!(
        names(
            &list(
                ListOptions::default()
                    .labels("tier=b")
                    .fields(format!("spec.nodeName={node}"))
            )
            .await
        ),
        ["bound-b"]
    );
    // Selectors survive pagination.
    let paged = resources
        .list_all(
            &pod_gvk(),
            Some(ns.name()),
            &ListOptions::default().labels("tier=a").limit(1),
        )
        .await
        .expect("paged selector list");
    assert_eq!(names(&paged), ["a-1", "a-2", "bound-a"]);
}

#[tokio::test]
async fn typed_and_dynamic_paths_return_identical_resources() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let dynamic = adapter(&client);
    let node = node_name(&dynamic).await;
    create_pods(
        &client,
        ns.name(),
        vec![
            pending_pod("golden-pending", &[("app", "golden")]),
            bound_pod("golden-bound", &[("app", "golden")], &node),
        ],
    )
    .await;
    let typed_config = |managed| ResourcesConfig {
        access_path: AccessPath::Typed,
        get_managed_fields: managed,
        ..ResourcesConfig::default()
    };
    let typed = adapter_with(&client, typed_config(ManagedFields::Keep));

    // Pods are mutated by the scheduler and kubelet while we read; compare reads of the
    // same stored object by pinning each pair to one resource version.
    for name in ["golden-pending", "golden-bound"] {
        let (d, t) = wait_until("both paths to read the same version", DEADLINE, || async {
            let d = dynamic
                .get(&pod_gvk(), Some(ns.name()), name)
                .await
                .expect("dynamic get");
            let t = typed
                .get(&pod_gvk(), Some(ns.name()), name)
                .await
                .expect("typed get");
            (t.meta.resource_version == d.meta.resource_version).then_some((d, t))
        })
        .await;
        assert_same(&t, &d, name);
        assert!(
            d.get("/metadata/managedFields").is_some(),
            "get keeps managedFields"
        );
    }

    let by_label = ListOptions::default().labels("app=golden");
    let typed_list = adapter_with(&client, typed_config(ManagedFields::Keep))
        .list_all(&pod_gvk(), Some(ns.name()), &by_label)
        .await
        .expect("typed list");
    let dynamic_list = dynamic
        .list_all(&pod_gvk(), Some(ns.name()), &by_label)
        .await
        .expect("dynamic list");
    for (t, d) in typed_list.items.iter().zip(&dynamic_list.items) {
        if t.meta.resource_version == d.meta.resource_version {
            assert_same(t, d, t.name());
        }
        assert!(
            d.get("/metadata/managedFields").is_none(),
            "list strips managedFields"
        );
        assert!(t.get("/metadata/managedFields").is_none());
    }
}

#[tokio::test]
async fn get_get_opt_and_cluster_scoped_kinds() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let resources = adapter(&client);
    create_pods(
        &client,
        ns.name(),
        vec![pending_pod("only", &[("app", "one")])],
    )
    .await;

    let pod = resources
        .get(&pod_gvk(), Some(ns.name()), "only")
        .await
        .expect("get");
    assert_eq!(pod.kind, pod_gvk());
    assert_eq!(pod.namespace(), Some(ns.name()));
    assert_eq!(pod.meta.labels.get("app").map(|v| &**v), Some("one"));

    let missing = resources
        .get(&pod_gvk(), Some(ns.name()), "ghost")
        .await
        .unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound);
    assert!(missing.message().contains("ghost"), "{missing}");
    assert!(
        resources
            .get_opt(&pod_gvk(), Some(ns.name()), "ghost")
            .await
            .expect("get_opt")
            .is_none()
    );

    let namespace = resources
        .get(&Gvk::new("", "v1", "Namespace"), None, ns.name())
        .await
        .expect("get namespace");
    assert_eq!(namespace.namespace(), None);
    let namespaces = resources
        .list(
            &Gvk::new("", "v1", "Namespace"),
            None,
            &ListOptions::default(),
        )
        .await
        .expect("list namespaces");
    assert!(namespaces.items.iter().any(|n| n.name() == ns.name()));

    let unknown = resources
        .list(
            &Gvk::new("nope.example.io", "v1", "Ghost"),
            None,
            &ListOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(unknown.kind(), ErrorKind::Unsupported);
}
