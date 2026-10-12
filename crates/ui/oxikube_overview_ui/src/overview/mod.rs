//! The cluster overview (E13-S09): the landing page of a cluster tab. Health tiles (nodes ready,
//! pods by phase), capacity and allocation bars (requests, limits and live usage against what the
//! nodes can allocate), the top CPU and memory pods and namespaces, the recent warnings, the
//! cluster's version, distribution, node count and API server; and, when a Prometheus-compatible
//! endpoint answers, the last hour of CPU and memory as graphs and sparklines.
//!
//! | File | Holds |
//! |---|---|
//! | `view` | [`ClusterOverview`], the workspace item; [`Shown`], what it draws, rebuilt per publish |
//! | `deps` | [`OverviewDeps`], [`HistoryDeps`] |
//! | `follow` | following the session; the live part started while shown and connected, stopped otherwise |
//! | `model` | the pure view-model: [`HealthTile`], [`CapacityLine`], [`TopRow`], the notices |
//! | `info` | the cluster-info card: [`VersionState`], [`InfoRows`], [`api_server_text`] (credentials cut out) |
//! | `history` | [`HistoryPanel`]: the Prometheus graphs, queried while shown |
//! | `render` | the frame and one file per card |
//!
//! # Data
//!
//! Every number comes from the session's `ResourceStore` (a cluster-wide Pod feed and the Node
//! feed, through the watch budget) and the app's `MetricsService`, never from ad hoc list calls:
//! [`OverviewFeed`](oxikube_app::cluster_overview::OverviewFeed) keeps the counts incrementally
//! on a background task and publishes a snapshot after each batch; the view rebuilds its
//! view-model from it and redraws, coalesced, at most ten times a second. A frame draws plain
//! `div`s from that model and the charts from their cached geometry.
//!
//! # When it runs
//!
//! The live part (the feed's driver, which owns the store subscriptions and the metrics
//! interests, the publish listener, the warnings timer, the version read and the history
//! queries) runs while the cluster is connected, the item is the displayed tab of its pane and
//! the cluster's tab is the one its window shows. Hiding either drops it: the feeds are released
//! (the store stops them after its grace period unless a table holds them) and metrics-server
//! stops being polled for it. The last numbers stay on screen.
//!
//! # Absence
//!
//! Without metrics-server, the usage bars and top lists say `metrics unavailable: <reason>` (a
//! click opens the cluster's metrics settings) and the tiles and allocation bars that do not need
//! metrics stay. Without a Prometheus endpoint the graphs card says why; without history metrics in
//! the app at all, no graphs are drawn. A forbidden feed says "no access", never zero.

mod deps;
mod follow;
mod history;
mod info;
mod model;
mod render;
mod view;

#[cfg(test)]
mod tests;

pub use deps::{HistoryDeps, OverviewDeps};
pub use history::{HISTORY_REFRESH, HistoryPanel, HistoryState, Sparks};
pub use info::{InfoRows, VersionState, api_server_text, info_rows};
pub use model::{
    CapacityLine, HealthTile, TopRow, capacity_lines, health_tiles, live_metrics_notice, top_rows,
};
pub use view::{ClusterOverview, OVERVIEW_ITEM_KEY, Shown, TopLists, WARNINGS_SHOWN};
