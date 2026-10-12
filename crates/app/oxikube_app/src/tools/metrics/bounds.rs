//! The limits that keep an agent's answer small and a call short.

use std::time::Duration;

/// Rows a `top_*` tool returns when `limit` is not given.
pub const DEFAULT_LIMIT: usize = 10;
/// Most rows a `top_*` tool returns.
pub const MAX_LIMIT: usize = 50;
/// Most series `query_promql` returns; the rest are counted, not listed.
pub const MAX_SERIES: usize = 20;
/// Most points of a series `query_promql` returns; a longer one is thinned evenly, keeping its
/// first and last point.
pub const MAX_POINTS_PER_SERIES: usize = 60;
/// Longest PromQL text, in characters.
pub const MAX_QUERY_CHARS: usize = 4_096;
/// Longest `range` of a range query.
pub const MAX_RANGE: Duration = Duration::from_secs(7 * 86_400);
/// Shortest `range` of a range query.
pub(super) const MIN_RANGE: Duration = Duration::from_secs(60);
/// How long a `top_*` call waits for the metrics API (and for the node list).
pub(super) const SNAPSHOT_DEADLINE: Duration = Duration::from_secs(15);
/// How long `query_promql` waits for Prometheus, on top of the adapter's own timeout.
pub(super) const QUERY_DEADLINE: Duration = Duration::from_secs(25);
