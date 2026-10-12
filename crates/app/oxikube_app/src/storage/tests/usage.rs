//! The claims columns: the Pods (used by) column over the shared reference index.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::Capabilities;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::storage::{pod_mounting, pod_mounting_in, pvc};

use crate::columns::{CellSort, ColumnId, ColumnProvider, CoreColumns};
use crate::references::ReferenceIndex;
use crate::storage::{PODS_COLUMN, PvcColumns};
use crate::store::{FeedState, RowChange, StoreDelta, StoreObject};

fn object(res: oxikube_domain::Resource) -> Arc<StoreObject> {
    Arc::new(StoreObject::Resource(res))
}

fn snapshot(pods: Vec<oxikube_domain::Resource>) -> StoreDelta {
    StoreDelta {
        rows: RowChange::Snapshot(pods.into_iter().map(object).collect()),
        state: FeedState::Ready,
        columns: None,
        len: 0,
        total: 0,
    }
}

fn provider(index: Arc<ReferenceIndex>) -> PvcColumns {
    PvcColumns::new(Arc::new(CoreColumns::new()), index)
}

fn claim_gvk() -> Gvk {
    Gvk::new("", "v1", "PersistentVolumeClaim")
}

#[test]
fn the_claims_table_lists_storage_class_size_status_and_pods() {
    let columns =
        provider(Arc::new(ReferenceIndex::new())).columns(&claim_gvk(), Capabilities::all());
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
    let index = Arc::new(ReferenceIndex::new());
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
    let provider = provider(Arc::new(ReferenceIndex::new()));
    let growing = object(pvc("data").requests("20Gi").capacity("10Gi").build());
    let now = Timestamp::from_second(1_800_000_000).unwrap();
    let size = provider.cell(&growing, &ColumnId::new("size"), now);
    assert_eq!(size.display(), "10Gi → 20Gi");
}

#[test]
fn a_finished_pod_no_longer_counts_and_a_claim_outside_the_scope_stays_blank() {
    use oxikube_domain::session::WatchScope;
    let index = Arc::new(ReferenceIndex::new());
    index.set_scope(WatchScope::Namespaces(vec!["shop".to_owned()]));
    index.apply(&snapshot(vec![
        pod_mounting("web-0", "shop", &["data"]),
        pod_mounting_in("job-0", "shop", &["data"], "Succeeded"),
    ]));
    let provider = provider(index);
    let column = ColumnId::new(PODS_COLUMN);
    let now = Timestamp::from_second(1_800_000_000).unwrap();
    let inside = object(pvc("data").namespace("shop").build());
    assert_eq!(provider.cell(&inside, &column, now).display(), "1");
    let outside = object(pvc("data").namespace("elsewhere").build());
    assert!(
        provider.cell(&outside, &column, now).is_pending(),
        "its Pods are not read: blank, not a false 0"
    );
}
