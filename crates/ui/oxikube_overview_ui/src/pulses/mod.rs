//! The pulses page (E13-S10): k9s' pulses for Oxikube. One tile per core kind and one each for
//! the cluster's CPU and memory, each with a number, a gauge and a short history, driven from the
//! keyboard and drilling into the matching table. Open it with `:pulses` (also `:pulse`, `:pu`),
//! the palette's "Show Pulses" (`cluster::ShowPulses`) or the sidebar's Metrics section.
//!
//! | File | Holds |
//! |---|---|
//! | `specs` | [`TileSpec`] and [`tile_specs`]: the sixteen tiles and the four-column grid |
//! | `ring` | [`PulseRing`]: the last 60 readings of a tile, for its sparkline |
//! | `face` | [`TileFace`]: the text, level and gauge of a tile for each state, pure |
//! | `model` | [`PulsesModel`]: the tiles' data, history and selection, with no window in it |
//! | `gauge`, `tile`, `grid` | the gauge bar, the tile element and the layout of the page |
//! | `view` | [`PulsesView`]: the workspace item, its render and its keys |
//! | `follow` | the session, the lease on the kinds, the tick |
//! | `gauges` | the cluster CPU and memory from the metrics service |
//! | `fallback` | the one-off count behind a kind that has no feed |
//! | `actions` | the key actions of the `Pulses` context |
//! | `commands` | `cluster::ShowPulses` and the window's [`PulsesViews`] that opens the page |
//! | `sidebar` | the sidebar entry |
//!
//! # What a tile shows
//!
//! A kind tile shows how many objects there are, and for the kinds with a health rule (pods,
//! nodes, deployments, stateful sets, daemon sets, jobs, cron jobs) how many are healthy as a
//! caption ("12 ready · 2 not ready") and a gauge; the others (namespaces, volumes, claims,
//! autoscalers, ingresses, network policies, service accounts) are counted and have no rule to
//! judge them by. The sparkline is the count over the last five minutes. The two cluster tiles
//! show metrics-server's use against the nodes' allocatable, toned by the cluster's
//! `metrics.thresholds` (warn 70, critical 90 by default), with the percentage as text.
//! A level is also a word and an icon, never colour alone.
//!
//! # Data, and what it costs
//!
//! Counts are read from the resource store's running tallies (O(1) per kind) through a
//! [`CountsLease`](oxikube_app::CountsLease): the store opens the kinds' feeds through the normal
//! path, so the watch budget decides, and a refused kind shows a one-off count fetched with a
//! metadata list (`limit=1`) marked "counted once" with `r` to count again. The page holds the lease
//! and a metrics interest only while it is the displayed tab, so a page in a background tab
//! costs nothing and the store and the metrics service stop on their own. The page reads once a
//! second, takes a history sample every [`HISTORY_INTERVAL`], and asks for a redraw (coalesced)
//! only when something changed. Tiles have a fixed size and never reflow.
//!
//! # Keyboard
//!
//! Arrows or `hjkl` move, `home` / `end` jump, `enter` opens the selected tile's table,
//! `r` counts again, `esc` closes the page (the tab before it shows again). A click selects a
//! tile and opens its table.

mod actions;
mod commands;
mod face;
mod fallback;
mod follow;
mod gauge;
mod gauges;
mod grid;
mod model;
mod ring;
mod sidebar;
mod specs;
mod tile;
mod view;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod tests;

use std::time::Duration;

pub use actions::{
    Close, Open, Refresh, SelectDown, SelectFirst, SelectLast, SelectLeft, SelectRight, SelectUp,
};
pub use commands::{PulsesRequestSink, PulsesViews, PulsesViewsDeps, register_commands};
pub use face::{ClusterFigure, Fallback, GaugeFace, Level, TileFace, cluster_face, kind_face};
pub use gauge::Gauge;
pub use model::{Move, PulsesModel, TileState};
pub use ring::{PULSE_SAMPLES, PulseRing};
pub use sidebar::{ENTRY_ID, register_sidebar};
pub use specs::{COLUMNS, Resource, TileSource, TileSpec, tile_specs};
pub use tile::{PulseTile, TILE_HEIGHT};
pub use view::{ITEM_KEY, PulsesDeps, PulsesView};

/// How often the page reads the store's counts.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// How often a history sample is taken: 60 samples are five minutes.
pub const HISTORY_INTERVAL: Duration = Duration::from_secs(5);

/// How often a kind with no feed is counted again.
pub const FALLBACK_INTERVAL: Duration = Duration::from_secs(60);

/// Ticks to wait before a kind that is merely not watched counts as one with no feed: the
/// lease opens its feeds on the store's spawner, a moment after it is made.
pub const SETTLE_TICKS: u32 = 3;

/// Registers the page's sidebar entry. Idempotent. Call it after `oxikube_settings_ui::init`,
/// which registers the section the entry joins.
pub fn init(cx: &mut gpui::App) {
    if !register_sidebar(cx) {
        tracing::warn!("the Metrics sidebar section is not registered: Pulses has no entry");
    }
}
