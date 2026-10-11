//! The Pods (used by) index and the claims columns.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::Capabilities;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::pod;
use oxikube_testkit::storage::{pod_mounting, pod_mounting_in, pvc};

use crate::columns::{CellSort, ColumnId, ColumnProvider, CoreColumns};
use crate::storage::{PODS_COLUMN, PvcColumns, PvcUsage, PvcUsageIndex, UsageStatus};
use crate::store::{FeedState, RowChange, RowOp, StoreDelta, StoreObject};

fn object(res: oxikube_domain::Resource) -> Arc<StoreObject> {
    Arc::new(StoreObject::Resource(res))
}

pub(super) fn delta(rows: RowChange, state: FeedState) -> StoreDelta {
    StoreDelta {
        rows,
        state,
        columns: None,
        len: 0,
        total: 0,
    }
}

fn snapshot(pods: Vec<oxikube_domain::Resource>) -> StoreDelta {
    delta(
        RowChange::Snapshot(pods.into_iter().map(object).collect()),
        FeedState::Ready,
    )
}

fn ops(ops: Vec<RowOp>) -> StoreDelta {
    delta(RowChange::Ops(ops), FeedState::Ready)
}

fn names(index: &PvcUsageIndex, ns: &str, claim: &str) -> Vec<String> {
    index
        .users(ns, claim)
        .iter()
        .map(|p| p.to_string())
        .collect()
}

#[test]
fn nothing_is_known_until_the_pods_are_listed() {
    let index = PvcUsageIndex::new();
    assert_eq!(index.status(), UsageStatus::Warming);
    assert_eq!(index.count("shop", "data"), None, "not a false zero");
    index.apply(&delta(RowChange::Unchanged, FeedState::Warming));
    assert_eq!(index.count("shop", "data"), None);
    index.apply(&snapshot(vec![]));
    assert_eq!(index.status(), UsageStatus::Ready);
    assert_eq!(index.count("shop", "data"), Some(0));
}

#[test]
fn a_snapshot_counts_the_pods_per_claim_and_namespace() {
    let index = PvcUsageIndex::new();
    index.apply(&snapshot(vec![
        pod_mounting("web-1", "shop", &["data"]),
        pod_mounting("web-0", "shop", &["data", "logs"]),
        pod_mounting("db-0", "other", &["data"]),
        pod().name("plain").namespace("shop").build(),
    ]));
    assert_eq!(index.count("shop", "data"), Some(2));
    assert_eq!(index.count("shop", "logs"), Some(1));
    assert_eq!(
        index.count("other", "data"),
        Some(1),
        "same name, other namespace"
    );
    assert_eq!(index.count("shop", "unused"), Some(0));
    assert_eq!(names(&index, "shop", "data"), ["web-0", "web-1"], "sorted");
    assert_eq!(index.pods(), 4);
}

#[test]
fn a_finished_pod_no_longer_holds_its_claim() {
    let index = PvcUsageIndex::new();
    index.apply(&snapshot(vec![
        pod_mounting("job-1", "shop", &["data"]),
        pod_mounting_in("job-0", "shop", &["data"], "Succeeded"),
        pod_mounting_in("job-2", "shop", &["data"], "Failed"),
    ]));
    assert_eq!(names(&index, "shop", "data"), ["job-1"]);
}

#[test]
fn ops_keep_the_index_current() {
    let index = PvcUsageIndex::new();
    index.apply(&snapshot(vec![
        pod_mounting("a", "shop", &["data"]),
        pod_mounting("c", "shop", &["data"]),
    ]));
    let before = index.version();

    // A new pod between the two.
    index.apply(&ops(vec![RowOp::Insert {
        index: 1,
        object: object(pod_mounting("b", "shop", &["data", "logs"])),
    }]));
    assert_eq!(names(&index, "shop", "data"), ["a", "b", "c"]);
    assert_eq!(names(&index, "shop", "logs"), ["b"]);
    assert!(index.version() > before);

    // `b` stops mounting `data` and mounts `cache` instead.
    index.apply(&ops(vec![RowOp::Update {
        index: 1,
        object: object(pod_mounting("b", "shop", &["logs", "cache"])),
    }]));
    assert_eq!(names(&index, "shop", "data"), ["a", "c"]);
    assert_eq!(names(&index, "shop", "cache"), ["b"]);

    // `a` is deleted.
    index.apply(&ops(vec![RowOp::Remove { index: 0 }]));
    assert_eq!(names(&index, "shop", "data"), ["c"]);
    assert_eq!(index.pods(), 2);

    // The last pod of a claim goes: the claim reads zero, not stale.
    index.apply(&ops(vec![
        RowOp::Remove { index: 1 },
        RowOp::Remove { index: 0 },
    ]));
    assert_eq!(index.count("shop", "data"), Some(0));
    assert_eq!(index.pods(), 0);
}

#[test]
fn a_pod_update_that_mounts_the_same_claims_changes_no_answer() {
    let index = PvcUsageIndex::new();
    index.apply(&snapshot(vec![pod_mounting("a", "shop", &["data"])]));
    let before = index.version();
    // Status churn: a new version of the pod, the same volumes.
    let busier = pod_mounting("a", "shop", &["data"]);
    index.apply(&ops(vec![RowOp::Update {
        index: 0,
        object: object(busier),
    }]));
    assert_eq!(index.version(), before, "views are not told to redraw");
    assert_eq!(index.count("shop", "data"), Some(1));
    // But a pod that finishes is a change.
    index.apply(&ops(vec![RowOp::Update {
        index: 0,
        object: object(pod_mounting_in("a", "shop", &["data"], "Succeeded")),
    }]));
    assert!(index.version() > before);
    assert_eq!(index.count("shop", "data"), Some(0));
}

#[test]
fn a_snapshot_replaces_everything() {
    let index = PvcUsageIndex::new();
    index.apply(&snapshot(vec![pod_mounting("a", "shop", &["data"])]));
    index.apply(&snapshot(vec![pod_mounting("z", "shop", &["logs"])]));
    assert_eq!(index.count("shop", "data"), Some(0));
    assert_eq!(names(&index, "shop", "logs"), ["z"]);
}

#[test]
fn pods_that_cannot_be_read_make_usage_unknown_with_the_reason() {
    let index = PvcUsageIndex::new();
    index.apply(&snapshot(vec![pod_mounting("a", "shop", &["data"])]));
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
    assert_eq!(index.count("shop", "data"), None);
    assert!(index.users("shop", "data").is_empty());
    // A feed that retries keeps its rows: usage stays known.
    index.apply(&delta(
        RowChange::Unchanged,
        FeedState::Retrying {
            message: "reset".into(),
        },
    ));
    assert_eq!(index.count("shop", "data"), Some(1));
}

fn provider(index: Arc<PvcUsageIndex>) -> PvcColumns {
    PvcColumns::new(Arc::new(CoreColumns::new()), index)
}

fn claim_gvk() -> Gvk {
    Gvk::new("", "v1", "PersistentVolumeClaim")
}

#[test]
fn the_claims_table_lists_storage_class_size_status_and_pods() {
    let columns =
        provider(Arc::new(PvcUsageIndex::new())).columns(&claim_gvk(), Capabilities::all());
    let ids: Vec<&str> = columns.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        &ids[..6],
        [
            "name",
            "namespace",
            "status",
            "storage-class",
            "size",
            PODS_COLUMN
        ]
    );
    let pods = columns.iter().find(|c| c.id == *PODS_COLUMN).unwrap();
    assert_eq!(&*pods.title, "Pods");
    assert!(!pods.wide);
}

#[test]
fn the_pods_cell_counts_and_is_blank_until_known() {
    let index = Arc::new(PvcUsageIndex::new());
    let provider = provider(index.clone());
    let claim = object(pvc("data").namespace("shop").build());
    let column = ColumnId::new(PODS_COLUMN);
    let now = Timestamp::from_second(1_800_000_000).unwrap();

    assert!(provider.cell(&claim, &column, now).is_pending());
    index.apply(&snapshot(vec![
        pod_mounting("web-0", "shop", &["data"]),
        pod_mounting("web-1", "shop", &["data"]),
    ]));
    let cell = provider.cell(&claim, &column, now);
    assert_eq!((cell.display(), cell.sort()), ("2", CellSort::Int(2)));
    let unused = object(pvc("scratch").namespace("shop").build());
    assert_eq!(provider.cell(&unused, &column, now).display(), "0");

    // The other columns are the catalogue's.
    let status = provider.cell(&claim, &ColumnId::new("status"), now);
    assert_eq!(status.display(), "Bound");
    let size = provider.cell(&claim, &ColumnId::new("size"), now);
    assert_eq!(size.display(), "10Gi");
}

#[test]
fn a_pending_resize_shows_both_sizes_in_the_size_column() {
    let provider = provider(Arc::new(PvcUsageIndex::new()));
    let growing = object(pvc("data").requests("20Gi").capacity("10Gi").build());
    let now = Timestamp::from_second(1_800_000_000).unwrap();
    let size = provider.cell(&growing, &ColumnId::new("size"), now);
    assert_eq!(size.display(), "10Gi → 20Gi");
}

#[test]
fn a_namespace_outside_the_scope_is_unknown_not_zero() {
    use oxikube_domain::session::WatchScope;
    let index = PvcUsageIndex::new();
    index.set_scope(WatchScope::Namespaces(vec!["default".to_owned()]));
    index.apply(&snapshot(vec![pod_mounting("web-0", "default", &["data"])]));
    assert_eq!(index.status(), UsageStatus::Ready);
    assert!(index.covers("default"));
    assert_eq!(index.count("default", "data"), Some(1));
    assert_eq!(index.count("default", "other"), Some(0));
    assert!(!index.covers("shop"));
    assert_eq!(
        index.count("shop", "data"),
        None,
        "shop's Pods are not read"
    );
    assert!(index.users("shop", "data").is_empty());
}

#[test]
fn a_new_scope_makes_the_index_warm_until_its_pods_are_listed() {
    use oxikube_domain::session::WatchScope;
    let index = PvcUsageIndex::new();
    index.apply(&snapshot(vec![pod_mounting("web-0", "default", &["data"])]));
    let before = index.version();
    index.set_scope(WatchScope::Namespaces(vec!["shop".to_owned()]));
    assert_eq!(index.status(), UsageStatus::Warming);
    assert_ne!(index.version(), before, "a view redraws");
    assert_eq!(
        index.count("shop", "data"),
        None,
        "not a zero from the old scope"
    );
    index.apply(&snapshot(vec![]));
    assert_eq!(index.count("shop", "data"), Some(0));
    // The same scope again changes nothing.
    let version = index.version();
    index.set_scope(WatchScope::Namespaces(vec!["shop".to_owned()]));
    assert_eq!(index.version(), version);
    assert_eq!(index.status(), UsageStatus::Ready);
}
