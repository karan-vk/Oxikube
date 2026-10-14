//! The muted clusters on disk: one `StatePort` value, `notifications.muted`, holding the ids of
//! the muted clusters and nothing else (no message, no title: ADR 0010, non-negotiable 5).

use std::collections::BTreeSet;

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{StateKey, StatePort};
use serde::{Deserialize, Serialize};

/// The state key the muted clusters are stored under.
pub const MUTED_KEY: &str = "notifications.muted";

/// The stored value. Versioned by shape: unknown fields are ignored, a value that does not parse
/// is read as "nothing muted" (with a warning), never an error that stops the centre.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    clusters: Vec<ClusterId>,
}

fn key() -> StateKey {
    StateKey::new(MUTED_KEY).expect("a valid state key")
}

/// The muted clusters stored in `state`.
///
/// # Errors
///
/// The port's error when the value cannot be read.
pub(crate) async fn load(state: &dyn StatePort) -> OxiResult<BTreeSet<ClusterId>> {
    let Some(value) = state.kv_get(&key()).await? else {
        return Ok(BTreeSet::new());
    };
    match serde_json::from_value::<Stored>(value) {
        Ok(stored) => Ok(stored.clusters.into_iter().collect()),
        Err(error) => {
            tracing::warn!(%error, key = MUTED_KEY, "unreadable muted clusters: none muted");
            Ok(BTreeSet::new())
        }
    }
}

/// Stores `muted` in `state`, replacing what was there.
///
/// # Errors
///
/// The port's error when the value cannot be written.
pub(crate) async fn save(state: &dyn StatePort, muted: &BTreeSet<ClusterId>) -> OxiResult<()> {
    let stored = Stored {
        clusters: muted.iter().cloned().collect(),
    };
    let value = serde_json::to_value(stored)
        .map_err(|error| oxikube_domain::OxiError::internal(error.to_string()))?;
    state.kv_set(&key(), value).await
}
