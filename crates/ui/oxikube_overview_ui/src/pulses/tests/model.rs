//! The model: the sixteen tiles, counts into faces, the history, and the keyboard order.

use oxikube_app::{CountState, FetchedCount, KindCount};

use crate::pulses::{
    COLUMNS, ClusterFigure, Fallback, Move, PULSE_SAMPLES, PulsesModel, TileSource, tile_specs,
};

fn counted(total: usize, rated: usize, healthy: usize) -> CountState {
    CountState::Counted(KindCount {
        total,
        rated,
        healthy,
    })
}

fn states(model: &PulsesModel, f: impl Fn(&str) -> CountState) -> Vec<CountState> {
    model
        .tiles()
        .iter()
        .filter(|t| matches!(t.spec.source, TileSource::Kind(_)))
        .map(|t| f(t.spec.id))
        .collect()
}

#[test]
fn the_page_has_the_two_gauges_and_the_fourteen_kinds_of_the_story() {
    let ids: Vec<&str> = tile_specs().iter().map(|s| s.id).collect();
    assert_eq!(
        ids,
        [
            "cluster-cpu",
            "cluster-memory",
            "nodes",
            "namespaces",
            "pods",
            "deployments",
            "statefulsets",
            "daemonsets",
            "jobs",
            "cronjobs",
            "persistentvolumes",
            "persistentvolumeclaims",
            "horizontalpodautoscalers",
            "ingresses",
            "networkpolicies",
            "serviceaccounts",
        ]
    );
    assert_eq!(ids.len() % COLUMNS, 0, "the grid has no ragged last row");
}

#[test]
fn every_tile_drills_into_a_table_of_its_own_kind() {
    for spec in tile_specs() {
        match &spec.source {
            TileSource::Kind(target) => assert_eq!(spec.drill, target.gvk, "{}", spec.id),
            TileSource::Cluster(_) => assert_eq!(spec.drill.kind.as_ref(), "Node", "{}", spec.id),
        }
    }
}

#[test]
fn targets_are_the_kind_tiles_in_order() {
    let model = PulsesModel::new();
    let targets = model.targets();
    assert_eq!(targets.len(), 14);
    assert_eq!(targets[0].gvk.kind.as_ref(), "Node");
    assert_eq!(targets[2].gvk.kind.as_ref(), "Pod");
}

#[test]
fn counts_land_on_their_tiles_and_report_whether_anything_changed() {
    let mut model = PulsesModel::new();
    let answers = states(&model, |id| match id {
        "pods" => counted(14, 14, 12),
        "persistentvolumes" => counted(3, 0, 0),
        _ => counted(0, 0, 0),
    });
    assert!(model.set_counts(&answers));
    assert!(
        !model.set_counts(&answers),
        "the same answers change nothing"
    );
    let pods = model.tile("pods").expect("pods");
    assert_eq!(pods.face().value, "14");
    assert_eq!(pods.face().caption, "12 healthy · 2 unhealthy");
    assert_eq!(
        model.tile("persistentvolumes").expect("pv").face().value,
        "3"
    );
}

#[test]
fn sampling_records_the_counted_totals_in_order_and_capped() {
    let mut model = PulsesModel::new();
    for round in 0..(PULSE_SAMPLES + 20) {
        let answers = states(&model, |id| {
            if id == "pods" {
                counted(round, round, round)
            } else {
                CountState::Loading
            }
        });
        model.set_counts(&answers);
        assert!(model.sample(1_000.0 + 5.0 * round as f64));
    }
    let pods = model.tile("pods").expect("pods");
    assert_eq!(pods.ring.len(), PULSE_SAMPLES, "capped");
    let values: Vec<f64> = pods.ring.iter().map(|p| p.v).collect();
    assert_eq!(values.first(), Some(&20.0), "the oldest 20 were dropped");
    assert_eq!(values.last(), Some(&((PULSE_SAMPLES + 19) as f64)));
    assert!(values.windows(2).all(|w| w[0] < w[1]), "ordered");
    // A kind that never had a number has no history, rather than a line of zeros.
    assert!(model.tile("jobs").expect("jobs").ring.is_empty());
}

#[test]
fn nothing_is_sampled_when_nothing_is_counted() {
    let mut model = PulsesModel::new();
    assert!(!model.sample(1.0));
}

#[test]
fn a_kind_the_store_cannot_count_wants_a_one_off_count() {
    let mut model = PulsesModel::new();
    let answers = states(&model, |id| match id {
        "pods" => CountState::OverBudget {
            message: "too many".into(),
        },
        "jobs" => CountState::Failed {
            message: "boom".into(),
        },
        "namespaces" => CountState::NoAccess {
            message: "forbidden".into(),
        },
        "nodes" => CountState::Loading,
        _ => counted(1, 0, 0),
    });
    model.set_counts(&answers);
    let wanting: Vec<&str> = model
        .wanting_fallback(true)
        .into_iter()
        .map(|i| model.tiles()[i].spec.id)
        .collect();
    assert_eq!(
        wanting,
        ["pods", "jobs"],
        "no access and loading are not counted once"
    );
}

#[test]
fn a_kind_not_watched_yet_waits_for_the_feeds_to_start() {
    let model = PulsesModel::new();
    assert!(model.wanting_fallback(false).is_empty());
    assert_eq!(model.wanting_fallback(true).len(), 14, "all are unwatched");
}

#[test]
fn a_one_off_count_is_sampled_and_dropped_when_the_feed_returns() {
    let mut model = PulsesModel::new();
    model.set_counts(&states(&model, |_| CountState::OverBudget {
        message: "m".into(),
    }));
    assert!(model.set_fallback(
        "pods",
        Fallback::Counted(FetchedCount {
            total: 42,
            exact: true
        })
    ));
    assert!(model.sample(10.0));
    assert_eq!(model.tile("pods").expect("pods").ring.latest(), Some(42.0));
    assert!(!model.set_fallback(
        "pods",
        Fallback::Counted(FetchedCount {
            total: 42,
            exact: true
        })
    ));
    // The feed comes back: the exact count wins and the stand-in is forgotten.
    model.set_counts(&states(&model, |_| counted(43, 43, 43)));
    assert_eq!(model.tile("pods").expect("pods").fallback, Fallback::None);
}

#[test]
fn the_cluster_history_replaces_the_ring_and_unchanged_data_changes_nothing() {
    let mut model = PulsesModel::new();
    let figure = ClusterFigure::Reading {
        used: 1,
        allocatable: 4,
        percent: Some(25.0),
        nodes: 1,
        nodes_reporting: 1,
    };
    let history = vec![(1.0, 10.0), (2.0, 25.0)];
    assert!(model.set_cluster("cluster-cpu", figure.clone(), history.clone()));
    assert!(!model.set_cluster("cluster-cpu", figure.clone(), history));
    let cpu = model.tile("cluster-cpu").expect("cpu");
    assert_eq!(cpu.ring.len(), 2);
    assert_eq!(cpu.face().value, "25%");
    assert!(!model.set_cluster("nope", figure, vec![]));
}

#[test]
fn the_selection_moves_on_the_grid_and_stops_at_its_edges() {
    let mut model = PulsesModel::new();
    let id = |m: &PulsesModel| m.selected_tile().spec.id;
    assert_eq!(id(&model), "cluster-cpu");
    assert!(!model.step(Move::Left), "the first column stays");
    assert!(!model.step(Move::Up), "the first row stays");
    assert!(model.step(Move::Right));
    assert_eq!(id(&model), "cluster-memory");
    assert!(model.step(Move::Down));
    assert_eq!(
        id(&model),
        "deployments",
        "one row down is `COLUMNS` tiles on"
    );
    assert!(model.step(Move::Up));
    assert!(model.step(Move::Last));
    assert_eq!(id(&model), "serviceaccounts");
    assert!(!model.step(Move::Right), "the last column stays");
    assert!(!model.step(Move::Down), "the last row stays");
    assert!(model.step(Move::Left));
    assert_eq!(id(&model), "networkpolicies");
    assert!(model.step(Move::First));
    assert_eq!(model.selected(), 0);
    assert!(!model.select(99), "out of range");
}

#[test]
fn walking_right_never_wraps_into_the_next_row() {
    let mut model = PulsesModel::new();
    for _ in 0..10 {
        model.step(Move::Right);
    }
    assert_eq!(model.selected(), COLUMNS - 1);
}
