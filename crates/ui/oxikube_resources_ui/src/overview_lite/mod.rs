//! The Workloads overview (E07-S11): a lightweight first screen for a connected cluster, built
//! from the `ResourceStore` alone. One card per workload kind (Pods, Deployments, DaemonSets,
//! StatefulSets, ReplicaSets, Jobs, CronJobs) with its total, how many are healthy and, since
//! E12-S07, a stacked status bar (Running / Pending / Failed..., Ready / Not ready) and a panel
//! of recent warnings. No metrics and no charts: those belong to the overview epic and E13.
//!
//! | File | Holds |
//! |---|---|
//! | `bars` | [`Segment`]s of a card's bar and the colour of each status bucket (pure) |
//! | `card` | a card: header, bar and legend; a click on a segment or legend entry filters the table |
//! | `warnings` | the recent-warnings panel over [`RecentWarnings`](oxikube_app::RecentWarnings) |
//! | `tiles` | [`Tile`] and the [`TileRegistry`], the extension point for later tiles |
//! | `face` | [`TileFace`], the pure map from a [`CountState`](oxikube_app::CountState) to text and tone |
//! | `view` | [`WorkloadsOverview`], the workspace item that draws the tiles |
//! | `follow` | following the session: the [`WorkloadCounters`](oxikube_app::WorkloadCounters), the warnings, the timer |
//!
//! # Data
//!
//! The view holds [`WorkloadCounters`](oxikube_app::WorkloadCounters), a [`CountsLease`](oxikube_app::CountsLease) for its tiles' kinds. The lease
//! opens each kind's feed through the store (so the watch budget decides: a refused kind shows a
//! dash with the reason, never a feed) and builds no row list. Numbers are read off the feeds'
//! running tallies once a second ([`REFRESH_INTERVAL`]) and the view redraws, coalesced, only
//! when a number changed. A forbidden kind says "no access", never zero. Closing the item drops
//! the lease; the store stops the feeds after its grace period unless a table holds them.
//!
//! # Navigation
//!
//! A click on a segment of a card's bar (or its legend entry) sends `resource::OpenList` and then
//! `table::SetFilter` with `-s <bucket>` (`-s pending`), the pair the `:` jump bar sends, so the
//! table opens already filtered to that status; the filter waits for a table that is not open yet.
//! A tile click sends `resource::OpenList { cluster, gvk }` through the window's
//! `CommandDispatcher` (the command bus in the app), the same command the sidebar entries and
//! the palette send; see [`navigate`](crate::navigate).
//!
//! # Extending
//!
//! Other crates add a tile with [`TileRegistry::register`] (a metrics tile, a chart) and every
//! open overview shows it. This story adds none.

mod bars;
mod card;
mod face;
mod follow;
mod tiles;
mod view;
mod warnings;

#[cfg(test)]
mod tests;

use std::time::Duration;

pub use bars::{Segment, colour, segments};
pub use face::{TileFace, face};
pub use tiles::{Tile, TileRegistry, workload_tiles};
pub use view::{OVERVIEW_ITEM_KEY, OverviewDeps, WorkloadsOverview};
pub use warnings::{WARNING_ROWS, headline};

/// How often the tiles read the store.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// Registers the workload tiles. Idempotent.
pub fn init(cx: &mut gpui::App) {
    for tile in workload_tiles() {
        TileRegistry::register(cx, tile);
    }
}
