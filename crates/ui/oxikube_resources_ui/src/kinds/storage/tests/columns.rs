//! What the storage tables list, and the claims' Pods column following the Pods feed.

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_app::ColumnsContext;
use oxikube_app::store::{FeedState, RowChange, StoreDelta, StoreObject};
use oxikube_testkit::storage::{pod_mounting, pod_mounting_in, pvc, storage_class};
use serde_json::json;

use super::{cell, kind, storage_window, visible};
use crate::kinds::storage::usage::usage_feed;
use crate::table::tests::fixture::cluster;

/// What the claims' column factory is given for the fixture's cluster.
fn context(f: &crate::table::tests::fixture::Fixture) -> ColumnsContext {
    let store = f
        .sessions
        .get(&cluster())
        .and_then(|session| f.deps.stores.for_session(&session));
    ColumnsContext {
        cluster: cluster(),
        store,
        sessions: f.sessions.clone(),
        shared: f.deps.columns.clone(),
    }
}

#[gpui::test]
fn the_claims_table_lists_class_size_status_and_the_pods_that_use_each_claim(
    cx: &mut TestAppContext,
) {
    let mut f = storage_window(
        cx,
        vec![
            pvc("data")
                .namespace("shop")
                .class("fast")
                .requests("10Gi")
                .build(),
            pvc("logs")
                .namespace("shop")
                .class("slow")
                .requests("1Gi")
                .capacity("1Gi")
                .build(),
            pvc("scratch").namespace("shop").pending().build(),
            pod_mounting("web-0", "shop", &["data"]),
            pod_mounting("web-1", "shop", &["data", "logs"]),
            pod_mounting_in("job-0", "shop", &["logs"], "Succeeded"),
        ],
    );
    let table = f.open(kind("persistentvolumeclaims"));
    f.settle();
    assert_eq!(f.names(&table), ["data", "logs", "scratch"]);
    assert_eq!(
        &visible(&mut f, &table)[..6],
        [
            "name",
            "namespace",
            "status",
            "storage-class",
            "size",
            "pods"
        ]
    );
    assert_eq!(cell(&mut f, &table, "data", "storage-class"), "fast");
    assert_eq!(cell(&mut f, &table, "data", "size"), "10Gi");
    assert_eq!(cell(&mut f, &table, "data", "status"), "Bound");
    assert_eq!(cell(&mut f, &table, "scratch", "status"), "Pending");
    // Two active pods mount `data`, one (the other finished) mounts `logs`, none `scratch`.
    assert_eq!(cell(&mut f, &table, "data", "pods"), "2");
    assert_eq!(cell(&mut f, &table, "logs", "pods"), "1");
    assert_eq!(cell(&mut f, &table, "scratch", "pods"), "0");
}

#[gpui::test]
fn the_pods_column_is_blank_not_zero_while_the_pods_cannot_be_read(cx: &mut TestAppContext) {
    let mut f = storage_window(cx, vec![pvc("data").namespace("shop").build()]);
    let table = f.open(kind("persistentvolumeclaims"));
    f.settle();
    assert_eq!(f.names(&table), ["data"]);
    assert_eq!(cell(&mut f, &table, "data", "pods"), "0");
    // The user may not list pods: usage is unknown, so the cell is empty rather than a false 0.
    let env = context(&f);
    let feed = usage_feed(&env).expect("a feed");
    feed.index().apply(&StoreDelta {
        rows: RowChange::Unchanged,
        state: FeedState::Forbidden {
            message: "pods is forbidden".into(),
        },
        columns: None,
        len: 0,
        total: 0,
    });
    assert_eq!(cell(&mut f, &table, "data", "pods"), "");
}

#[gpui::test]
fn the_pods_column_follows_the_pods_without_a_new_feed_per_cell(cx: &mut TestAppContext) {
    let mut f = storage_window(
        cx,
        vec![
            pvc("data").namespace("shop").build(),
            pod_mounting("web-0", "shop", &["data"]),
        ],
    );
    let table = f.open(kind("persistentvolumeclaims"));
    f.settle();
    assert_eq!(cell(&mut f, &table, "data", "pods"), "1");
    // Watches so far: the claims, and the pods the index reads.
    let watches = |f: &mut crate::table::tests::fixture::Fixture| {
        f.ports()
            .resources
            .recorded_calls()
            .iter()
            .filter(|c| matches!(c, oxikube_testkit::ResourceCall::Watch { .. }))
            .count()
    };
    let before = watches(&mut f);

    // Another pod mounts the claim: the index is told by the Pods feed, and the table redraws on
    // its next tick (usage is not part of a claim's own version).
    let env = context(&f);
    let feed = usage_feed(&env).expect("a connected cluster has a feed");
    let renders = f.vcx.update(|_, cx| table.read(cx).render_count());
    let both = vec![
        Arc::new(StoreObject::Resource(pod_mounting(
            "web-0",
            "shop",
            &["data"],
        ))),
        Arc::new(StoreObject::Resource(pod_mounting(
            "web-1",
            "shop",
            &["data"],
        ))),
    ];
    feed.index().apply(&StoreDelta {
        rows: RowChange::Snapshot(both),
        state: FeedState::Ready,
        columns: None,
        len: 2,
        total: 2,
    });
    f.vcx
        .executor()
        .advance_clock(std::time::Duration::from_secs(2));
    f.settle();
    assert_eq!(cell(&mut f, &table, "data", "pods"), "2");
    assert!(
        f.vcx.update(|_, cx| table.read(cx).render_count()) > renders,
        "the table redrew for the new count"
    );
    // Asking for the feed again shares the one that exists: no second Pods watch.
    let again = usage_feed(&env).unwrap();
    assert!(Arc::ptr_eq(&feed, &again));
    assert_eq!(watches(&mut f), before);
}

#[gpui::test]
fn the_pods_feed_stops_with_the_last_claim_view(cx: &mut TestAppContext) {
    let mut f = storage_window(
        cx,
        vec![
            pvc("data").namespace("shop").build(),
            pod_mounting("web-0", "shop", &["data"]),
        ],
    );
    let table = f.open(kind("persistentvolumeclaims"));
    f.settle();
    let live = |f: &mut crate::table::tests::fixture::Fixture| f.ports().resources.live_watches();
    assert_eq!(live(&mut f), 2, "claims and pods");
    // Closing the table drops its provider, the only holder of the feed.
    let tabs = f.tabs.clone();
    let id = table.entity_id();
    f.vcx.update(|window, cx| {
        let tab = tabs.read(cx).tab(&cluster()).cloned().unwrap();
        let workspace = tab.read(cx).workspace().clone();
        workspace.update(cx, |ws, cx| {
            ws.close_item(id, window, cx);
        });
    });
    drop(table);
    f.settle();
    f.clock.advance(std::time::Duration::from_secs(60));
    f.settle();
    assert_eq!(live(&mut f), 0, "nothing keeps a watch open");
}

#[gpui::test]
fn volumes_list_capacity_modes_reclaim_status_and_claim(cx: &mut TestAppContext) {
    let volume = oxikube_testkit::resource("v1", "PersistentVolume")
        .cluster_scoped()
        .name("pv-1")
        .field(
            "spec",
            json!({"capacity": {"storage": "100Gi"}, "accessModes": ["ReadWriteMany", "ReadOnlyMany"],
                   "persistentVolumeReclaimPolicy": "Retain", "storageClassName": "fast",
                   "claimRef": {"namespace": "shop", "name": "data"}, "volumeMode": "Block"}),
        )
        .field("status", json!({"phase": "Bound"}))
        .build();
    let mut f = storage_window(cx, vec![volume]);
    let table = f.open(kind("persistentvolumes"));
    f.settle();
    for (column, want) in [
        ("capacity", "100Gi"),
        ("access-modes", "RWX,ROX"),
        ("reclaim-policy", "Retain"),
        ("status", "Bound"),
        ("claim", "shop/data"),
    ] {
        assert_eq!(cell(&mut f, &table, "pv-1", column), want, "{column}");
    }
}

#[gpui::test]
fn classes_list_the_provisioner_and_a_default_badge(cx: &mut TestAppContext) {
    let mut f = storage_window(
        cx,
        vec![
            storage_class("fast")
                .expandable(true)
                .default_class()
                .build(),
            storage_class("slow").build(),
            storage_class("legacy").beta_default_class().build(),
        ],
    );
    let table = f.open(kind("storageclasses"));
    f.settle();
    assert_eq!(
        cell(&mut f, &table, "fast", "provisioner"),
        "ebs.csi.aws.com"
    );
    assert_eq!(cell(&mut f, &table, "fast", "default"), "default");
    assert_eq!(
        cell(&mut f, &table, "legacy", "default"),
        "default",
        "the beta annotation counts"
    );
    assert_eq!(cell(&mut f, &table, "slow", "default"), "");
    assert_eq!(cell(&mut f, &table, "fast", "allow-expansion"), "true");
}

#[gpui::test]
fn snapshots_and_their_classes_list_source_class_readiness_size_and_time(cx: &mut TestAppContext) {
    let snapshot = oxikube_testkit::resource("snapshot.storage.k8s.io/v1", "VolumeSnapshot")
        .name("nightly")
        .namespace("shop")
        .field(
            "spec",
            json!({"source": {"persistentVolumeClaimName": "data"}, "volumeSnapshotClassName": "csi-snap"}),
        )
        .field(
            "status",
            json!({"readyToUse": true, "restoreSize": "10Gi", "creationTime": "2026-02-01T03:00:00Z"}),
        )
        .build();
    let class = oxikube_testkit::resource("snapshot.storage.k8s.io/v1", "VolumeSnapshotClass")
        .cluster_scoped()
        .name("csi-snap")
        .annotation("snapshot.storage.kubernetes.io/is-default-class", "true")
        .field("driver", json!("ebs.csi.aws.com"))
        .field("deletionPolicy", json!("Delete"))
        .build();
    let mut f = storage_window(cx, vec![snapshot, class]);
    let snapshots = f.open(kind("volumesnapshots"));
    f.settle();
    for (column, want) in [
        ("ready", "true"),
        ("source-pvc", "data"),
        ("snapshot-class", "csi-snap"),
        ("restore-size", "10Gi"),
        ("created", "2026-02-01T03:00:00Z"),
    ] {
        assert_eq!(
            cell(&mut f, &snapshots, "nightly", column),
            want,
            "{column}"
        );
    }
    let classes = f.open(kind("volumesnapshotclasses"));
    f.settle();
    assert_eq!(
        cell(&mut f, &classes, "csi-snap", "driver"),
        "ebs.csi.aws.com"
    );
    assert_eq!(
        cell(&mut f, &classes, "csi-snap", "deletion-policy"),
        "Delete"
    );
    assert_eq!(cell(&mut f, &classes, "csi-snap", "default"), "default");
}
