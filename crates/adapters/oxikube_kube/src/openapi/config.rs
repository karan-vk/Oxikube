//! [`OpenApiConfig`]: the deadlines of [`OpenApiSchemas`](super::OpenApiSchemas).

use std::time::Duration;

/// Default per-request deadline for the index and group-document fetches.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

/// Deadline for the `/version` read that keys the disk cache.
pub(super) const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// How old the in-memory index must be before a miss re-reads it (a CRD that
/// was just created reaches the server's OpenAPI document a moment after its
/// discovery event).
pub const DEFAULT_REFRESH_ON_MISS_SECS: u64 = 5;

/// Settings of [`OpenApiSchemas`](super::OpenApiSchemas).
#[derive(Debug, Clone)]
pub struct OpenApiConfig {
    /// Deadline for one index or group-document fetch.
    pub request_timeout: Duration,
    /// A lookup that finds no schema re-reads the index first when the one in
    /// memory is at least this old, so a kind added since is found without an
    /// explicit invalidate. Younger indexes answer `NotFound` immediately.
    pub refresh_on_miss_after: Duration,
}

impl Default for OpenApiConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
            refresh_on_miss_after: Duration::from_secs(DEFAULT_REFRESH_ON_MISS_SECS),
        }
    }
}
