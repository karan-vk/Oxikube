//! [`PulsesModel`]: everything the page shows, as plain data with no window in it.
//!
//! One [`TileState`] per [`TileSpec`]: its latest answer from the store (or the metrics), its
//! [`PulseRing`] of history and its cached [`TileFace`]. The view feeds the model from the store's
//! counts once a second and from the metrics service on each poll, and draws what the model
//! holds; navigation is index arithmetic on the fixed grid. Nothing here touches gpui, so the
//! counts, the breakdown, the capped history and the keyboard order are unit-tested directly.
//!
//! A setter returns whether anything changed, so the view redraws (coalesced) only then.

use oxikube_app::columns::UsageThresholds;
use oxikube_app::{CountState, CountTarget};

use super::face::{ClusterFigure, Fallback, TileFace, cluster_face, kind_face};
use super::ring::PulseRing;
use super::specs::{COLUMNS, TileSource, TileSpec, tile_specs};

/// A key press that moves the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// One tile left (`h`, left arrow); stays in its row.
    Left,
    /// One tile right (`l`, right arrow); stays in its row.
    Right,
    /// One row up (`k`, up arrow).
    Up,
    /// One row down (`j`, down arrow).
    Down,
    /// The first tile (`home`).
    First,
    /// The last tile (`end`).
    Last,
}

/// What the page holds for one tile.
#[derive(Debug)]
pub struct TileState {
    /// What the tile is.
    pub spec: TileSpec,
    /// The store's answer, for a kind tile (`NotWatched` until the first read).
    pub count: CountState,
    /// The one-off count behind a kind with no feed.
    pub fallback: Fallback,
    /// The metrics' answer, for a gauge tile.
    pub figure: ClusterFigure,
    /// The recent readings of the tile's number.
    pub ring: PulseRing,
    face: TileFace,
}

impl TileState {
    fn new(spec: TileSpec) -> Self {
        let mut tile = Self {
            count: CountState::NotWatched,
            fallback: Fallback::None,
            figure: ClusterFigure::Waiting,
            ring: PulseRing::new(),
            face: TileFace::default(),
            spec,
        };
        tile.face = tile.compute_face(&UsageThresholds::default());
        tile
    }

    /// What the tile says now.
    pub fn face(&self) -> &TileFace {
        &self.face
    }

    fn compute_face(&self, thresholds: &UsageThresholds) -> TileFace {
        match &self.spec.source {
            TileSource::Kind(_) => kind_face(self.spec.id, &self.count, &self.fallback),
            TileSource::Cluster(resource) => cluster_face(*resource, &self.figure, thresholds),
        }
    }

    /// The count to record in the history now: from the feed, else from the one-off count.
    pub(super) fn sampled_value(&self) -> Option<f64> {
        match (&self.count, &self.fallback) {
            (CountState::Counted(count), _) => Some(count.total as f64),
            (_, Fallback::Counted(count)) => Some(count.total as f64),
            _ => None,
        }
    }

    /// Whether the kind has no feed to count from, so a one-off count stands in for it.
    /// `settled` is whether the feeds had time to start: a kind that is merely not watched yet is
    /// not asked for before that.
    pub fn wants_fallback(&self, settled: bool) -> bool {
        matches!(self.spec.source, TileSource::Kind(_))
            && match self.count {
                CountState::OverBudget { .. } | CountState::Failed { .. } => true,
                CountState::NotWatched => settled,
                _ => false,
            }
    }
}

/// The whole page. See the [module docs](self).
#[derive(Debug)]
pub struct PulsesModel {
    tiles: Vec<TileState>,
    selected: usize,
    thresholds: UsageThresholds,
}

impl Default for PulsesModel {
    fn default() -> Self {
        Self::new()
    }
}

impl PulsesModel {
    /// A model of the sixteen tiles of [`tile_specs`], nothing known yet, the first selected.
    pub fn new() -> Self {
        Self {
            tiles: tile_specs().into_iter().map(TileState::new).collect(),
            selected: 0,
            thresholds: UsageThresholds::default(),
        }
    }

    /// The tiles in page order.
    pub fn tiles(&self) -> &[TileState] {
        &self.tiles
    }

    /// The tiles, mutably: the view takes each one's sparkline `Arc` while it draws.
    pub fn tiles_mut(&mut self) -> &mut [TileState] {
        &mut self.tiles
    }

    /// The tile `id`.
    pub fn tile(&self, id: &str) -> Option<&TileState> {
        self.tiles.iter().find(|t| t.spec.id == id)
    }

    /// The kinds to count, in the order [`set_counts`](Self::set_counts) takes answers.
    pub fn targets(&self) -> Vec<CountTarget> {
        self.tiles
            .iter()
            .filter_map(|t| t.spec.target().cloned())
            .collect()
    }

    /// Takes the store's answers, one per [`targets`](Self::targets) entry. Returns whether any
    /// tile's answer changed.
    pub fn set_counts(&mut self, states: &[CountState]) -> bool {
        let thresholds = self.thresholds;
        let mut changed = false;
        let kinds = self
            .tiles
            .iter_mut()
            .filter(|t| matches!(t.spec.source, TileSource::Kind(_)));
        for (tile, state) in kinds.zip(states) {
            if &tile.count == state {
                continue;
            }
            tile.count = state.clone();
            // A kind that has a feed again no longer needs its one-off count.
            if matches!(tile.count, CountState::Counted(_)) {
                tile.fallback = Fallback::None;
            }
            tile.face = tile.compute_face(&thresholds);
            changed = true;
        }
        changed
    }

    /// Takes the one-off count of the kind tile `id`. Returns whether it changed.
    pub fn set_fallback(&mut self, id: &str, fallback: Fallback) -> bool {
        let thresholds = self.thresholds;
        let Some(tile) = self.tiles.iter_mut().find(|t| t.spec.id == id) else {
            return false;
        };
        if tile.fallback == fallback {
            return false;
        }
        tile.fallback = fallback;
        tile.face = tile.compute_face(&thresholds);
        true
    }

    /// Forgets every one-off count, so the kinds that still lack a feed ask again.
    pub fn clear_fallbacks(&mut self) -> bool {
        let thresholds = self.thresholds;
        let mut changed = false;
        for tile in &mut self.tiles {
            if tile.fallback != Fallback::None {
                tile.fallback = Fallback::None;
                tile.face = tile.compute_face(&thresholds);
                changed = true;
            }
        }
        changed
    }

    /// Takes what the metrics say of the cluster's `resource` and its history (oldest first,
    /// seconds since the epoch and percent). Returns whether the tile changed.
    pub fn set_cluster(
        &mut self,
        id: &str,
        figure: ClusterFigure,
        history: Vec<(f64, f64)>,
    ) -> bool {
        let thresholds = self.thresholds;
        let Some(tile) = self.tiles.iter_mut().find(|t| t.spec.id == id) else {
            return false;
        };
        let same_history = tile.ring.len() == history.len()
            && tile
                .ring
                .iter()
                .zip(&history)
                .all(|(point, (t, v))| point.t == *t && point.v == *v);
        if tile.figure == figure && same_history {
            return false;
        }
        tile.figure = figure;
        if !same_history {
            tile.ring.replace(history);
        }
        tile.face = tile.compute_face(&thresholds);
        true
    }

    /// The thresholds the gauges are toned by (the cluster's `metrics.thresholds`). Returns
    /// whether the faces changed.
    pub fn set_thresholds(&mut self, thresholds: UsageThresholds) -> bool {
        if self.thresholds == thresholds {
            return false;
        }
        self.thresholds = thresholds;
        for tile in &mut self.tiles {
            tile.face = tile.compute_face(&thresholds);
        }
        true
    }

    /// Records every counted kind's total in its history at `at` (seconds since the epoch).
    /// Returns whether any history moved.
    pub fn sample(&mut self, at: f64) -> bool {
        let mut moved = false;
        for tile in &mut self.tiles {
            if let Some(value) = tile.sampled_value() {
                tile.ring.push(at, value);
                moved = true;
            }
        }
        moved
    }

    /// The kind tiles a one-off count stands in for (see [`TileState::wants_fallback`] for
    /// `settled`).
    pub fn wanting_fallback(&self, settled: bool) -> Vec<usize> {
        self.tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| t.wants_fallback(settled))
            .map(|(i, _)| i)
            .collect()
    }

    /// The selected tile's index.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The selected tile.
    pub fn selected_tile(&self) -> &TileState {
        &self.tiles[self.selected]
    }

    /// Selects tile `index`. Returns whether the selection changed (`false` out of range).
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.tiles.len() || index == self.selected {
            return false;
        }
        self.selected = index;
        true
    }

    /// Moves the selection one step on the grid; a step off the edge stays put. Returns whether
    /// the selection changed.
    pub fn step(&mut self, movement: Move) -> bool {
        let (row, column) = (self.selected / COLUMNS, self.selected % COLUMNS);
        let last = self.tiles.len() - 1;
        let target = match movement {
            Move::Left if column > 0 => self.selected - 1,
            Move::Right if column + 1 < COLUMNS && self.selected < last => self.selected + 1,
            Move::Up if row > 0 => self.selected - COLUMNS,
            Move::Down if self.selected + COLUMNS <= last => self.selected + COLUMNS,
            Move::First => 0,
            Move::Last => last,
            _ => return false,
        };
        self.select(target)
    }
}
