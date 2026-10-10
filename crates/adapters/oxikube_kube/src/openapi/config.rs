//! [`OpenApiConfig`]: the deadlines of [`OpenApiSchemas`](super::OpenApiSchemas).

use std::time::Duration;

/// Default per-request deadline for the index and group-document fetches.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

/// Deadline for the `/version` read that keys the disk cache.
pub(super) const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// Deadline for a settle re-check's index read. Shorter than the request deadline: the re-check
/// sits in front of every lookup, memory hits included, so a hung server must not hold them for
/// the full request timeout.
pub(super) const RECHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// How old the in-memory index must be before a miss re-reads it (a CRD that
/// was just created reaches the server's OpenAPI document a moment after its
/// discovery event).
pub const DEFAULT_REFRESH_ON_MISS_SECS: u64 = 5;

/// How long after an `invalidate` the index may still be the old one: the API server publishes
/// an edited CRD's document a fraction of a second after the CRD watch reports the edit
/// (measured up to about 0.9 s on a kind cluster).
pub const DEFAULT_SETTLE_AFTER_INVALIDATE_MILLIS: u64 = 3000;

/// How often, while settling, a lookup re-reads the index.
pub const DEFAULT_RECHECK_EVERY_MILLIS: u64 = 1000;

/// Settings of [`OpenApiSchemas`](super::OpenApiSchemas).
#[derive(Debug, Clone)]
pub struct OpenApiConfig {
    /// Deadline for one index or group-document fetch.
    pub request_timeout: Duration,
    /// A lookup that finds no schema re-reads the index first when the one in
    /// memory is at least this old, so a kind added since is found without an
    /// explicit invalidate. Younger indexes answer `NotFound` immediately.
    pub refresh_on_miss_after: Duration,
    /// After an `invalidate`, lookups keep comparing the index with the cache until one read
    /// was made this long after it, so a schema cached from an index that still lagged the
    /// change is replaced (the first lookup after the invalidate cannot tell).
    pub settle_after_invalidate: Duration,
    /// While settling, the youngest index a lookup re-reads: at most one index request per
    /// this interval, a failed one included.
    pub recheck_every: Duration,
}

impl Default for OpenApiConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
            refresh_on_miss_after: Duration::from_secs(DEFAULT_REFRESH_ON_MISS_SECS),
            settle_after_invalidate: Duration::from_millis(DEFAULT_SETTLE_AFTER_INVALIDATE_MILLIS),
            recheck_every: Duration::from_millis(DEFAULT_RECHECK_EVERY_MILLIS),
        }
    }
}
