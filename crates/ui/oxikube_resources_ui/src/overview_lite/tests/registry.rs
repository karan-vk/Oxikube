//! The tile registry: the seven workload tiles, and the extension point.

use gpui::TestAppContext;
use oxikube_app::CountTarget;

use crate::overview_lite::{Tile, TileRegistry, init, workload_tiles};

#[test]
fn the_workload_tiles_are_the_seven_kinds_in_order() {
    let titles: Vec<String> = workload_tiles()
        .iter()
        .map(|t| t.title.to_string())
        .collect();
    assert_eq!(
        titles,
        [
            "Deployments",
            "StatefulSets",
            "DaemonSets",
            "ReplicaSets",
            "Jobs",
            "CronJobs",
            "Pods"
        ]
    );
    let kinds: Vec<String> = workload_tiles()
        .iter()
        .map(|t| t.target.gvk.kind.to_string())
        .collect();
    assert_eq!(
        kinds,
        [
            "Deployment",
            "StatefulSet",
            "DaemonSet",
            "ReplicaSet",
            "Job",
            "CronJob",
            "Pod"
        ]
    );
}

#[gpui::test]
fn init_registers_them_and_a_later_tile_slots_in_by_order(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        init(cx);
        let ids: Vec<String> = TileRegistry::tiles(cx)
            .iter()
            .map(|t| t.id.to_string())
            .collect();
        assert_eq!(ids.len(), 7, "idempotent");
        assert_eq!(ids.first().map(String::as_str), Some("deployments"));
        assert_eq!(ids.last().map(String::as_str), Some("pods"));

        // A metrics tile of a later epic registers itself between two of ours.
        let target = CountTarget::core("", "nodes").expect("nodes");
        TileRegistry::register(
            cx,
            Tile {
                id: "nodes".into(),
                title: "Nodes".into(),
                target: target.clone(),
                order: 150,
            },
        );
        let ids: Vec<String> = TileRegistry::tiles(cx)
            .iter()
            .map(|t| t.id.to_string())
            .collect();
        assert_eq!(ids[2], "nodes");
        // The same id replaces it.
        TileRegistry::register(
            cx,
            Tile {
                id: "nodes".into(),
                title: "Cluster nodes".into(),
                target,
                order: 150,
            },
        );
        let tiles = TileRegistry::tiles(cx);
        assert_eq!(tiles.len(), 8);
        assert_eq!(tiles[2].title.as_ref(), "Cluster nodes");
    });
}
