//! The payload of the `cloud::*` commands (E18-S06): how a refresh of cloud discovery is asked
//! for.

use serde::{Deserialize, Serialize};

/// Whether `cloud::Refresh` may answer from the discovery cache.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CloudRefreshMode {
    /// Show the cache at once and run the provider's CLI only when the cache is older than its
    /// TTL (what opening the Cloud section asks for).
    #[default]
    IfStale,
    /// Run the provider's CLI even when the cache is fresh (the user pressed Refresh).
    Force,
}
