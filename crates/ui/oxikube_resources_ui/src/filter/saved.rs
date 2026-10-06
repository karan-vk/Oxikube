//! The saved filter of one kind, in the [`StatePort`] under `table.filter.<group>/<Kind>`.
//!
//! Like the column layout it is per kind, not per cluster, and only written while
//! `resource_table.persist_filter` is on. The row holds the text typed in the bar, nothing from
//! the cluster. Reads and writes are async and never run on the UI thread; the writes go through
//! one background task per table that keeps only the newest text of a burst.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, AppContext as _};
use oxikube_domain::ids::Gvk;
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ports::{StateKey, StatePort, StatePortExt as _};
use serde::{Deserialize, Serialize};

use crate::table::kind_key;

/// The version this build writes.
pub const FILTER_VERSION: u32 = 1;

/// The state key prefix of saved filters.
pub const FILTER_PREFIX: &str = "table.filter.";

#[derive(Serialize, Deserialize)]
struct Row {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    text: String,
}

/// The state key of `gvk`'s filter: `table.filter.<group>/<Kind>` (`core` for the core group).
///
/// # Errors
///
/// `Validation` when the kind's name makes an invalid state key.
pub fn filter_key(gvk: &Gvk) -> OxiResult<StateKey> {
    kind_key(FILTER_PREFIX, gvk)
}

/// Reads and writes one kind's saved filter text.
#[derive(Clone)]
pub struct SavedFilter {
    state: Arc<dyn StatePort>,
    key: StateKey,
}

impl SavedFilter {
    /// The saved filter of `gvk`.
    ///
    /// # Errors
    ///
    /// `Validation` when the kind's name makes an invalid state key.
    pub fn new(state: Arc<dyn StatePort>, gvk: &Gvk) -> OxiResult<Self> {
        Ok(Self {
            state,
            key: filter_key(gvk)?,
        })
    }

    /// The saved text; `None` when there is none. A row of another shape or a newer version
    /// reads as `None`: a bad row must never break the table.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn load(&self) -> OxiResult<Option<String>> {
        match self.state.kv_get_as::<Row>(&self.key).await {
            Ok(Some(row)) if row.version <= FILTER_VERSION => Ok(Some(row.text)),
            Ok(_) => Ok(None),
            Err(error) if error.kind() == ErrorKind::Validation => {
                tracing::warn!(%error, "saved filter is unreadable: ignored");
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Writes `text`, replacing the saved filter.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn save(&self, text: &str) -> OxiResult<()> {
        let row = Row {
            version: FILTER_VERSION,
            text: text.to_owned(),
        };
        self.state.kv_set_as(&self.key, &row).await
    }
}

impl std::fmt::Debug for SavedFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedFilter")
            .field("key", &self.key.as_str())
            .finish_non_exhaustive()
    }
}

/// Sends filter texts to the one background task that saves them, in order; a burst collapses
/// into one write of the newest. The task is detached on purpose: it ends when the table (the
/// sender) goes, after writing what was queued, so a filter typed right before closing is kept.
/// It holds no entity.
pub struct FilterWriter {
    tx: UnboundedSender<String>,
}

impl FilterWriter {
    /// Starts the writer task for `saved`.
    pub fn spawn(saved: SavedFilter, cx: &App) -> Self {
        let (tx, mut rx) = unbounded::<String>();
        cx.background_spawn(async move {
            while let Some(mut text) = rx.next().await {
                while let Ok(newer) = rx.try_recv() {
                    text = newer;
                }
                if let Err(error) = saved.save(&text).await {
                    tracing::warn!(%error, "saving the filter failed");
                }
            }
        })
        .detach();
        Self { tx }
    }

    /// Queues `text` to be written.
    pub fn save(&self, text: String) {
        // The task outlives every sender, so this only fails once it is gone.
        self.tx.unbounded_send(text).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_per_kind_with_core_for_the_empty_group() {
        let key = |g: Gvk| filter_key(&g).unwrap().as_str().to_owned();
        assert_eq!(key(Gvk::new("", "v1", "Pod")), "table.filter.core/Pod");
        assert_eq!(
            key(Gvk::new("apps", "v1", "Deployment")),
            "table.filter.apps/Deployment"
        );
        assert_eq!(
            key(Gvk::new("apps", "v1beta2", "Deployment")),
            "table.filter.apps/Deployment",
            "versions share the filter"
        );
    }
}
