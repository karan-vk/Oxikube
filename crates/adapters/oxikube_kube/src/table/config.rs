//! Tuning for the Table API feed.

use std::time::Duration;

/// Settings for [`TableFeedPort`](oxikube_ports::TableFeedPort) on
/// [`KubeResources`](crate::KubeResources), carried in
/// [`ResourcesConfig::table`](crate::ResourcesConfig::table).
///
/// The defaults follow sofka's proven Table loop (research 2.3a): 30 s refresh, 15 s request
/// timeout, 5 s retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableConfig {
    /// How often a feed re-lists when it cannot watch: the kind has no `watch` verb, RBAC
    /// denies the watch, the server answers a Table watch with plain objects, or rows carry
    /// no object to key them by (`IncludeObject::None`). Each re-list is diffed, so unchanged
    /// rows are not re-sent. Default 30 s.
    pub refresh_interval: Duration,
    /// Client-side deadline for one list request (one page). Default 15 s.
    pub request_timeout: Duration,
    /// Pause after a retryable failure (list or watch) before the feed tries again.
    /// Default 5 s.
    pub retry_delay: Duration,
    /// Page size of a feed's (re-)list when `ListOptions::limit` is unset. Default 500.
    pub page_size: u32,
    /// Server-side watch timeout (`timeoutSeconds`). When it elapses the feed re-watches
    /// from the last resource version without re-listing. Default 290 s (kube's default).
    pub watch_timeout_secs: u32,
    /// Most watch events coalesced into one [`TableBatch`](oxikube_ports::TableBatch): the
    /// events already buffered when the feed wakes, up to this many. Default 512.
    pub max_batch: usize,
}

impl Default for TableConfig {
    fn default() -> Self {
        Self {
            refresh_interval: Duration::from_secs(30),
            request_timeout: Duration::from_secs(15),
            retry_delay: Duration::from_secs(5),
            page_size: 500,
            watch_timeout_secs: 290,
            max_batch: 512,
        }
    }
}
