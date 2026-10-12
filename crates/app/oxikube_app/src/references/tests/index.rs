//! The index itself: each reference kind is extracted, deltas keep it correct, status and scope
//! make an unknown answer unknown rather than zero.

use oxikube_domain::session::WatchScope;
use oxikube_domain::view::RefKind;
use serde_json::json;

use super::{delta, object, ops, owned_by, pod_cm, pod_in, pod_with, ready, snapshot};
use crate::references::{ReferenceIndex, Usage, UsageStatus};
use crate::store::{FeedState, RowChange, RowOp};

fn names(index: &ReferenceIndex, kind: RefKind, ns: Option<&str>, name: &str) -> Vec<String> {
    match index.usage(kind, ns, name) {
        Usage::Pods { pods, .. } => pods.iter().map(|p| p.pod.name.to_string()).collect(),
        other => panic!("expected pods, got {other:?}"),
    }
}

const SHOP: Option<&str> = Some("shop");

#[test]
fn nothing_is_known_until_the_pods_are_listed() {
    let index = ReferenceIndex::new();
    assert_eq!(index.status(), UsageStatus::Warming);
    assert_eq!(index.usage(RefKind::ConfigMap, SHOP, "cfg"), Usage::Warming);
    assert_eq!(
        index.count(RefKind::ConfigMap, SHOP, "cfg"),
        None,
        "not a false zero"
    );
    index.apply(&delta(RowChange::Unchanged, FeedState::Warming));
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), None);
    index.apply(&snapshot(vec![]));
    assert_eq!(index.status(), UsageStatus::Ready);
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), Some(0));
}

#[test]
fn every_kind_of_reference_is_found_from_a_pod_spec() {
    let spec = json!({
        "serviceAccountName": "web-sa",
        "priorityClassName": "urgent",
        "imagePullSecrets": [{"name": "regcred"}],
        "volumes": [
            {"name": "data", "persistentVolumeClaim": {"claimName": "data"}},
            {"name": "cfg", "configMap": {"name": "vol-cm"}},
            {"name": "sec", "secret": {"secretName": "vol-secret"}},
            {"name": "proj", "projected": {"sources": [
                {"configMap": {"name": "proj-cm"}}, {"secret": {"name": "proj-secret"}}
            ]}},
        ],
        "containers": [{"name": "c",
            "env": [
                {"name": "A", "valueFrom": {"configMapKeyRef": {"name": "env-cm", "key": "k"}}},
                {"name": "B", "valueFrom": {"secretKeyRef": {"name": "env-secret", "key": "k"}}},
            ],
            "envFrom": [{"configMapRef": {"name": "from-cm"}}, {"secretRef": {"name": "from-secret"}}],
        }],
        "initContainers": [{"name": "i", "envFrom": [{"configMapRef": {"name": "init-cm"}}]}],
        "ephemeralContainers": [{"name": "e", "env": [
            {"name": "X", "valueFrom": {"secretKeyRef": {"name": "debug-secret", "key": "k"}}}
        ]}],
    });
    let index = ready(vec![pod_with("web-0", "shop", spec)]);
    let found = |kind, ns, name: &str| names(&index, kind, ns, name) == ["web-0"];
    assert!(found(RefKind::ServiceAccount, SHOP, "web-sa"));
    assert!(
        found(RefKind::PriorityClass, None, "urgent"),
        "cluster-scoped: no namespace"
    );
    assert!(found(RefKind::PersistentVolumeClaim, SHOP, "data"));
    for cm in ["vol-cm", "proj-cm", "env-cm", "from-cm", "init-cm"] {
        assert!(found(RefKind::ConfigMap, SHOP, cm), "{cm}");
    }
    for secret in [
        "regcred",
        "vol-secret",
        "proj-secret",
        "env-secret",
        "from-secret",
        "debug-secret",
    ] {
        assert!(found(RefKind::Secret, SHOP, secret), "{secret}");
    }
    // A name is looked up in its own kind and namespace only.
    assert_eq!(
        names(&index, RefKind::Secret, SHOP, "vol-cm"),
        Vec::<String>::new()
    );
    assert_eq!(
        names(&index, RefKind::ConfigMap, Some("other"), "vol-cm"),
        Vec::<String>::new()
    );
}

#[test]
fn a_snapshot_counts_pods_per_object_and_namespace_sorted() {
    let index = ready(vec![
        pod_cm("web-1", "shop", "cfg"),
        pod_cm("web-0", "shop", "cfg"),
        pod_cm("db-0", "other", "cfg"),
        pod_with("plain", "shop", json!({})),
    ]);
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), Some(2));
    assert_eq!(
        index.count(RefKind::ConfigMap, Some("other"), "cfg"),
        Some(1),
        "same name, other namespace"
    );
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "unused"), Some(0));
    assert_eq!(
        names(&index, RefKind::ConfigMap, SHOP, "cfg"),
        ["web-0", "web-1"],
        "sorted"
    );
    assert_eq!(index.pods(), 4);
    assert_eq!(
        index.indexed(),
        3,
        "a pod that names nothing is not indexed"
    );
}

#[test]
fn a_priority_class_is_used_across_namespaces() {
    let class = json!({"priorityClassName": "urgent"});
    let index = ready(vec![
        pod_with("a", "shop", class.clone()),
        pod_with("b", "other", class),
    ]);
    match index.usage(RefKind::PriorityClass, None, "urgent") {
        Usage::Pods { pods, partial } => {
            assert_eq!(pods.len(), 2);
            assert!(!partial, "every namespace is read");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(index.count(RefKind::PriorityClass, None, "urgent"), Some(2));
}

#[test]
fn a_finished_pod_no_longer_holds_what_it_named() {
    let spec = json!({"volumes": [{"name": "v", "configMap": {"name": "cfg"}}]});
    let index = ready(vec![
        pod_with("job-1", "shop", spec.clone()),
        pod_in("job-0", "shop", spec.clone(), "Succeeded"),
        pod_in("job-2", "shop", spec, "Failed"),
    ]);
    assert_eq!(names(&index, RefKind::ConfigMap, SHOP, "cfg"), ["job-1"]);
}

#[test]
fn ops_keep_the_index_current() {
    let index = ready(vec![pod_cm("a", "shop", "cfg"), pod_cm("c", "shop", "cfg")]);
    let before = index.version();

    // A new pod between the two.
    let b = pod_with(
        "b",
        "shop",
        json!({"volumes": [{"name": "v", "configMap": {"name": "cfg"}}, {"name": "w", "secret": {"secretName": "tls"}}]}),
    );
    index.apply(&ops(vec![RowOp::Insert {
        index: 1,
        object: object(b),
    }]));
    assert_eq!(
        names(&index, RefKind::ConfigMap, SHOP, "cfg"),
        ["a", "b", "c"]
    );
    assert_eq!(names(&index, RefKind::Secret, SHOP, "tls"), ["b"]);
    assert!(index.version() > before);

    // `b` stops using `cfg` (the reference is removed) and uses `other` instead of `tls`.
    let b2 = pod_with(
        "b",
        "shop",
        json!({"volumes": [{"name": "w", "secret": {"secretName": "other"}}]}),
    );
    index.apply(&ops(vec![RowOp::Update {
        index: 1,
        object: object(b2),
    }]));
    assert_eq!(names(&index, RefKind::ConfigMap, SHOP, "cfg"), ["a", "c"]);
    assert_eq!(
        names(&index, RefKind::Secret, SHOP, "tls"),
        Vec::<String>::new()
    );
    assert_eq!(names(&index, RefKind::Secret, SHOP, "other"), ["b"]);

    // `a` is deleted.
    index.apply(&ops(vec![RowOp::Remove { index: 0 }]));
    assert_eq!(names(&index, RefKind::ConfigMap, SHOP, "cfg"), ["c"]);
    assert_eq!(index.pods(), 2);

    // The last pods of an object go: it reads zero, not stale.
    index.apply(&ops(vec![
        RowOp::Remove { index: 1 },
        RowOp::Remove { index: 0 },
    ]));
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), Some(0));
    assert_eq!(index.pods(), 0);
    assert_eq!(index.indexed(), 0);
}

#[test]
fn a_pod_update_that_names_the_same_objects_changes_no_answer() {
    let index = ready(vec![pod_cm("a", "shop", "cfg")]);
    let before = index.version();
    // Status churn: a new version of the pod, the same spec.
    index.apply(&ops(vec![RowOp::Update {
        index: 0,
        object: object(pod_cm("a", "shop", "cfg")),
    }]));
    assert_eq!(index.version(), before, "views are not told to redraw");
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), Some(1));
    // But a pod that finishes is a change.
    let spec = json!({"volumes": [{"name": "v", "configMap": {"name": "cfg"}}]});
    index.apply(&ops(vec![RowOp::Update {
        index: 0,
        object: object(pod_in("a", "shop", spec, "Succeeded")),
    }]));
    assert!(index.version() > before);
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), Some(0));
}

#[test]
fn a_snapshot_replaces_everything() {
    let index = ready(vec![pod_cm("a", "shop", "cfg")]);
    index.apply(&snapshot(vec![pod_cm("z", "shop", "other")]));
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), Some(0));
    assert_eq!(names(&index, RefKind::ConfigMap, SHOP, "other"), ["z"]);
}

#[test]
fn a_pods_owner_is_kept_for_grouping() {
    let rs = owned_by(
        pod_cm("web-0", "shop", "cfg"),
        "apps/v1",
        "ReplicaSet",
        "web-5d",
    );
    let index = ready(vec![rs, pod_cm("lone", "shop", "cfg")]);
    let Usage::Pods { pods, .. } = index.usage(RefKind::ConfigMap, SHOP, "cfg") else {
        panic!("pods");
    };
    let owner = |name: &str| {
        pods.iter()
            .find(|p| &*p.pod.name == name)
            .unwrap()
            .owner
            .clone()
    };
    assert_eq!(
        owner("web-0").map(|o| o.name.to_string()),
        Some("web-5d".into())
    );
    assert_eq!(owner("lone"), None);
    // The owner changes with no change of references: the index follows.
    let rs2 = owned_by(
        pod_cm("web-0", "shop", "cfg"),
        "apps/v1",
        "ReplicaSet",
        "web-6e",
    );
    index.apply(&ops(vec![RowOp::Update {
        index: 1,
        object: object(rs2),
    }]));
    let Usage::Pods { pods, .. } = index.usage(RefKind::ConfigMap, SHOP, "cfg") else {
        panic!("pods");
    };
    assert!(
        pods.iter()
            .any(|p| p.owner.as_ref().is_some_and(|o| &*o.name == "web-6e"))
    );
}

#[test]
fn pods_that_cannot_be_read_make_usage_unknown_with_the_reason() {
    let index = ready(vec![pod_cm("a", "shop", "cfg")]);
    index.apply(&delta(
        RowChange::Unchanged,
        FeedState::Forbidden {
            message: "pods is forbidden".into(),
        },
    ));
    assert_eq!(
        index.status(),
        UsageStatus::Unavailable("pods is forbidden".into())
    );
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), None);
    assert_eq!(
        index.usage(RefKind::ConfigMap, SHOP, "cfg"),
        Usage::Unavailable("pods is forbidden".into())
    );
    // A feed that retries keeps its rows: usage stays known.
    index.apply(&delta(
        RowChange::Unchanged,
        FeedState::Retrying {
            message: "reset".into(),
        },
    ));
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), Some(1));
}

#[test]
fn a_namespace_outside_the_scope_is_unknown_not_zero() {
    let index = ReferenceIndex::new();
    index.set_scope(WatchScope::Namespaces(vec!["default".to_owned()]));
    index.apply(&snapshot(vec![pod_cm("web-0", "default", "cfg")]));
    assert_eq!(index.status(), UsageStatus::Ready);
    assert!(index.covers("default"));
    assert_eq!(
        index.count(RefKind::ConfigMap, Some("default"), "cfg"),
        Some(1)
    );
    assert_eq!(
        index.count(RefKind::ConfigMap, Some("default"), "other"),
        Some(0)
    );
    assert!(!index.covers("shop"));
    assert_eq!(
        index.count(RefKind::ConfigMap, SHOP, "cfg"),
        None,
        "shop's Pods are not read"
    );
    assert_eq!(
        index.usage(RefKind::ConfigMap, SHOP, "cfg"),
        Usage::OutsideScope
    );
    assert_eq!(
        index.usage(RefKind::ConfigMap, None, "cfg"),
        Usage::OutsideScope
    );
}

#[test]
fn a_priority_class_is_partial_when_only_some_namespaces_are_read() {
    let index = ReferenceIndex::new();
    index.set_scope(WatchScope::Namespaces(vec!["default".to_owned()]));
    index.apply(&snapshot(vec![pod_with(
        "web-0",
        "default",
        json!({"priorityClassName": "urgent"}),
    )]));
    match index.usage(RefKind::PriorityClass, None, "urgent") {
        Usage::Pods { pods, partial } => {
            assert_eq!(pods.len(), 1);
            assert!(partial, "pods elsewhere may use it too");
        }
        other => panic!("{other:?}"),
    }
    assert!(!index.covers_cluster());
}

#[test]
fn a_new_scope_makes_the_index_warm_until_its_pods_are_listed() {
    let index = ready(vec![pod_cm("web-0", "default", "cfg")]);
    let before = index.version();
    index.set_scope(WatchScope::Namespaces(vec!["shop".to_owned()]));
    assert_eq!(index.status(), UsageStatus::Warming);
    assert_ne!(index.version(), before, "a view redraws");
    assert_eq!(
        index.count(RefKind::ConfigMap, SHOP, "cfg"),
        None,
        "not a zero from the old scope"
    );
    index.apply(&snapshot(vec![]));
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), Some(0));
    // The same scope again changes nothing.
    let version = index.version();
    index.set_scope(WatchScope::Namespaces(vec!["shop".to_owned()]));
    assert_eq!(index.version(), version);
    assert_eq!(index.status(), UsageStatus::Ready);
}
