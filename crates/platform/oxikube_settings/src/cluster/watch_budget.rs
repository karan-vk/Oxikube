//! The `watch_budget` block (E04-F543): limits on the feeds the app opens on a cluster.
//!
//! Like `node_shell`, the block's fields merge across layers: `default.json` sets every field,
//! the user's top-level block may change some, and a cluster's own block others.

use std::time::Duration;

use oxikube_ports::WatchBudgetPrefs;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The watch budget, as one settings layer writes it. Every field is optional so a layer sets
/// any subset; unset fields keep the built-in defaults.
///
/// Example, a tighter budget for one very large cluster:
///
/// ```json
/// "clusters": {
///   "3f2a9c1b7d4e8a60": { "watch_budget": { "max_feeds": 24, "max_objects": 50000 } }
/// }
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WatchBudgetContent {
    /// Most feeds open at once on the cluster. Each namespace of a multi-namespace selection is
    /// its own feed. At the limit, feeds of views closed less than `idle_grace_seconds` ago
    /// are closed first; then the new view says the budget is full.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub max_feeds: Option<usize>,
    /// No new feed opens while the open feeds hold this many objects in total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub max_objects: Option<u64>,
    /// While the open feeds hold this many objects, a kind that would get whole objects gets a
    /// metadata-only feed instead (names, labels and ages, no spec or status). Set it to
    /// `max_objects` or more to never do that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_above: Option<u64>,
    /// How long a feed nobody looks at keeps running, in seconds, so switching back to a view
    /// is instant. `0` stops it at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_grace_seconds: Option<u64>,
}

impl From<WatchBudgetContent> for WatchBudgetPrefs {
    /// Unset fields keep the defaults; a limit of zero is read as one (a budget that admits no
    /// feed at all would leave every view empty).
    fn from(content: WatchBudgetContent) -> Self {
        let default = WatchBudgetPrefs::default();
        Self {
            max_feeds: content.max_feeds.unwrap_or(default.max_feeds).max(1),
            max_objects: content.max_objects.unwrap_or(default.max_objects).max(1),
            metadata_above: content.metadata_above.unwrap_or(default.metadata_above),
            idle_grace: content
                .idle_grace_seconds
                .map_or(default.idle_grace, Duration::from_secs),
        }
    }
}
