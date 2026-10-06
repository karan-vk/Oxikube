//! [`Tile`] and the [`TileRegistry`]: what the overview shows, registered rather than hard-coded
//! so the metrics and chart tiles of later epics add themselves.

use gpui::{App, Global, SharedString};
use oxikube_app::CountTarget;
use oxikube_app::store::WORKLOAD_TARGETS;

/// One tile: a kind to count.
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    /// Stable id (`pods`); a tile registered with an existing id replaces it.
    pub id: SharedString,
    /// The heading.
    pub title: SharedString,
    /// The kind counted, and the kind a click opens.
    pub target: CountTarget,
    /// Sort key: lower first. Ties keep registration order.
    pub order: u32,
}

/// The registered tiles. A GPUI global; use the associated functions with the `App`.
#[derive(Clone, Debug, Default)]
pub struct TileRegistry {
    tiles: Vec<Tile>,
}

impl Global for TileRegistry {}

impl TileRegistry {
    /// Registers `tile`; one with the same id is replaced in place. Open overviews redraw.
    pub fn register(cx: &mut App, tile: Tile) {
        let registry = cx.default_global::<TileRegistry>();
        match registry.tiles.iter_mut().find(|t| t.id == tile.id) {
            Some(existing) => *existing = tile,
            None => registry.tiles.push(tile),
        }
    }

    /// The tiles, ordered by `order`, ties in registration order.
    pub fn tiles(cx: &App) -> Vec<Tile> {
        let mut tiles = cx
            .try_global::<TileRegistry>()
            .map(|r| r.tiles.clone())
            .unwrap_or_default();
        tiles.sort_by_key(|t| t.order);
        tiles
    }
}

/// The titles of [`WORKLOAD_TARGETS`], in the same order.
const TITLES: [&str; 7] = [
    "Deployments",
    "StatefulSets",
    "DaemonSets",
    "ReplicaSets",
    "Jobs",
    "CronJobs",
    "Pods",
];

/// The seven workload tiles: Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs,
/// Pods.
pub fn workload_tiles() -> Vec<Tile> {
    WORKLOAD_TARGETS
        .iter()
        .zip(TITLES)
        .enumerate()
        .filter_map(|(ix, ((group, plural), title))| {
            Some(Tile {
                id: (*plural).to_owned().into(),
                title: title.into(),
                target: CountTarget::core(group, plural)?,
                order: u32::try_from(ix).unwrap_or(u32::MAX) * 100,
            })
        })
        .collect()
}
