//! The panel's data and the owned rows, as pure functions (no window).

use std::collections::HashSet;
use std::sync::Arc;

use oxikube_app::CoreColumns;
use oxikube_app::columns::Tone;
use oxikube_app::store::StoreObject;
use oxikube_domain::Resource;
use oxikube_domain::view::WorkloadKind;
use serde_json::json;

use super::{web_deployment, web_pod, web_replicaset};
use crate::kinds::workloads::panel::owned::{owned_by, pod_rows, replicaset_rows, uids};
use crate::kinds::workloads::panel::{MAX_ROWS, PanelModel};
use crate::table::tests::fixture::cluster;

fn object(resource: Resource) -> Arc<StoreObject> {
    Arc::new(StoreObject::Resource(resource))
}

#[test]
fn a_statefulset_shows_its_partition_and_service() {
    let sts = Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "StatefulSet",
        "metadata": {"name": "db", "namespace": "shop", "uid": "u", "resourceVersion": "4"},
        "spec": {"replicas": 3, "serviceName": "db-headless", "selector": {"matchLabels": {"app": "db"}},
                 "updateStrategy": {"type": "RollingUpdate", "rollingUpdate": {"partition": 2}},
                 "template": {"spec": {"containers": [{"name": "pg", "image": "postgres:17"}]}}},
        "status": {"replicas": 3, "readyReplicas": 3, "updatedReplicas": 1}
    }))
    .unwrap();
    let model = PanelModel::build(&sts).unwrap();
    assert_eq!(model.kind, WorkloadKind::StatefulSet);
    let stats: Vec<_> = model
        .stats
        .iter()
        .map(|s| (s.label, s.value.as_str(), s.tone))
        .collect();
    assert_eq!(
        stats,
        [
            ("Ready", "3/3", Tone::Ok),
            ("Updated", "1", Tone::Warn),
            ("Replicas", "3", Tone::Neutral),
            ("Partition", "2", Tone::Warn),
        ]
    );
    assert_eq!(
        model.strategy.as_deref(),
        Some("RollingUpdate · Partition 2")
    );
    assert!(
        model
            .extras
            .contains(&("Service", "db-headless".to_owned()))
    );
}

#[test]
fn a_replication_controller_shows_ready_current_and_desired_and_no_strategy() {
    let rc = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ReplicationController",
        "metadata": {"name": "legacy", "namespace": "shop", "uid": "u", "resourceVersion": "1"},
        "spec": {"replicas": 2, "selector": {"app": "legacy"}},
        "status": {"replicas": 2, "readyReplicas": 1}
    }))
    .unwrap();
    let model = PanelModel::build(&rc).unwrap();
    assert_eq!(model.kind, WorkloadKind::ReplicationController);
    let stats: Vec<_> = model
        .stats
        .iter()
        .map(|s| (s.label, s.value.as_str()))
        .collect();
    assert_eq!(
        stats,
        [("Ready", "1/2"), ("Current", "2"), ("Desired", "2")]
    );
    assert_eq!(model.strategy, None);
    assert_eq!(model.selector, "app=legacy");
}

#[test]
fn a_metadata_only_object_or_another_kind_has_no_model() {
    assert!(PanelModel::build(&web_deployment()).is_some());
    let partial = web_deployment().into_partial();
    assert!(partial.is_partial());
    assert!(PanelModel::build(&partial).is_none());
    assert!(PanelModel::build(&web_pod("p", "o", "web")).is_none());
}

#[test]
fn ownership_is_by_uid_not_by_label() {
    let feed = vec![
        object(web_pod("a", "rs-1", "web")),
        object(web_pod("b", "rs-2", "web")),
        object(web_pod("c", "rs-1", "other")),
    ];
    let owners: HashSet<Arc<str>> = HashSet::from([Arc::from("rs-1")]);
    let names: Vec<_> = owned_by(&owners, &feed)
        .iter()
        .map(|o| o.name().to_owned())
        .collect();
    assert_eq!(names, ["a", "c"]);
    assert!(owned_by(&HashSet::new(), &feed).is_empty());
    let sets = vec![object(web_replicaset("rs", "rs-uid", 1, 1, "d"))];
    assert_eq!(uids(&sets), HashSet::from([Arc::<str>::from("rs-uid")]));
}

#[test]
fn replicasets_list_the_running_revision_first_then_old_ones_newest_first() {
    let mut old = web_replicaset("web-1", "u1", 0, 0, "d");
    old.meta.creation = Some("2026-01-01T00:00:00Z".parse().unwrap());
    let mut older = web_replicaset("web-0", "u0", 0, 0, "d");
    older.meta.creation = Some("2025-12-01T00:00:00Z".parse().unwrap());
    let mut current = web_replicaset("web-2", "u2", 3, 3, "d");
    current.meta.creation = Some("2026-02-01T00:00:00Z".parse().unwrap());
    let owned = [older, current, old].map(object);
    let rows = replicaset_rows(&cluster(), &owned);
    let names: Vec<_> = rows.rows.iter().map(|r| &*r.target.name).collect();
    assert_eq!(names, ["web-2", "web-1", "web-0"]);
    assert_eq!(rows.rows[0].status, "3/3");
    assert_eq!(rows.rows[0].tone, Tone::Ok);
    assert_eq!(
        rows.rows[1].tone,
        Tone::Neutral,
        "a scaled-down revision is not a problem"
    );
}

#[test]
fn a_big_workload_lists_a_page_and_counts_the_rest() {
    let owned: Vec<_> = (0..MAX_ROWS + 15)
        .map(|n| object(web_pod(&format!("web-{n:03}"), "rs", "web")))
        .collect();
    let rows = pod_rows(
        &cluster(),
        &CoreColumns::new(),
        &owned,
        "2026-01-02T00:00:00Z".parse().unwrap(),
    );
    assert_eq!(
        (rows.rows.len(), rows.total, rows.hidden()),
        (MAX_ROWS, MAX_ROWS + 15, 15)
    );
    assert_eq!(&*rows.rows[0].target.name, "web-000", "by name");
    assert_eq!(rows.rows[0].status, "Running");
    assert_eq!(rows.rows[0].info, "1/1");
}

#[test]
fn a_huge_workload_picks_its_page_by_name_without_costing_every_pod() {
    const PODS: usize = 5_000;
    // Scrambled input (a prime stride visits every index once), so the page is a selection and
    // not just the head of the list.
    let owned: Vec<_> = (0..PODS)
        .map(|n| {
            let i = (n * 3_001) % PODS;
            object(web_pod(&format!("web-{i:05}"), "rs", "web"))
        })
        .collect();
    let started = std::time::Instant::now();
    let rows = pod_rows(
        &cluster(),
        &CoreColumns::new(),
        &owned,
        "2026-01-02T00:00:00Z".parse().unwrap(),
    );
    let took = started.elapsed();
    assert_eq!((rows.rows.len(), rows.total), (MAX_ROWS, PODS));
    let names: Vec<_> = rows
        .rows
        .iter()
        .map(|r| r.target.name.to_string())
        .collect();
    let expected: Vec<_> = (0..MAX_ROWS).map(|i| format!("web-{i:05}")).collect();
    assert_eq!(names, expected, "the smallest names, in order");
    assert!(
        rows.rows
            .iter()
            .all(|r| r.status == "Running" && r.info == "1/1")
    );
    // Cells are computed for the page only; walking 5k objects to select it is far under this
    // (computing two cells for each of the 5k pods was more than twice it in a debug build).
    assert!(
        took < std::time::Duration::from_millis(100),
        "5000 pods took {took:?}"
    );
}
