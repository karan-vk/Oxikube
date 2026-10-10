//! The saved filter of one view, in the [`StatePort`] under
//! `table.filter.<cluster>/<group>/<Kind>`.
//!
//! A view is a kind in a cluster (E11-S06): `app=x` typed in the Pods of one cluster is not the
//! filter of the Pods of another. The row is only written while `resource_table.persist_filter`
//! is on and holds the text typed in the bar, nothing from the cluster; clearing the filter
//! (`escape`, the chip) removes the row. Reads and writes are async and never run on the UI
//! thread; the writes go through one background task per table that keeps only the newest text
//! of a burst.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, AppContext as _};
use oxikube_domain::ids::{ClusterId, Gvk};
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

/// The state key of the filter of `gvk` in `cluster`:
/// `table.filter.<cluster>/<group>/<Kind>` (`core` for the core group).
///
/// # Errors
///
/// `Validation` when the kind's name makes an invalid state key.
pub fn filter_key(cluster: &ClusterId, gvk: &Gvk) -> OxiResult<StateKey> {
    kind_key(&format!("{FILTER_PREFIX}{cluster}/"), gvk)
}

/// Reads and writes one view's saved filter text.
#[derive(Clone)]
pub struct SavedFilter {
    state: Arc<dyn StatePort>,
    key: StateKey,
}

impl SavedFilter {
    /// The saved filter of `gvk` in `cluster`.
    ///
    /// # Errors
    ///
    /// `Validation` when the kind's name makes an invalid state key.
    pub fn new(state: Arc<dyn StatePort>, cluster: &ClusterId, gvk: &Gvk) -> OxiResult<Self> {
        Ok(Self {
            state,
            key: filter_key(cluster, gvk)?,
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

    /// Removes the saved filter.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn clear(&self) -> OxiResult<()> {
        self.state.kv_delete(&self.key).await.map(drop)
    }

    /// Writes `text`, replacing the saved filter; no text removes it.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn save(&self, text: &str) -> OxiResult<()> {
        if text.is_empty() {
            return self.clear().await;
        }
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
    use oxikube_domain::ids::ContextName;

    use super::*;

    fn cluster(context: &str) -> ClusterId {
        ClusterId::new("tests", &ContextName::from(context))
    }

    #[test]
    fn the_key_is_per_cluster_and_kind_with_core_for_the_empty_group() {
        let dev = cluster("dev");
        let key = |c: &ClusterId, g: Gvk| filter_key(c, &g).unwrap().as_str().to_owned();
        assert_eq!(
            key(&dev, Gvk::new("", "v1", "Pod")),
            format!("table.filter.{dev}/core/Pod")
        );
        assert_eq!(
            key(&dev, Gvk::new("apps", "v1", "Deployment")),
            format!("table.filter.{dev}/apps/Deployment")
        );
        assert_eq!(
            key(&dev, Gvk::new("apps", "v1beta2", "Deployment")),
            key(&dev, Gvk::new("apps", "v1", "Deployment")),
            "versions share the filter"
        );
        let pod = Gvk::new("", "v1", "Pod");
        assert_ne!(
            key(&dev, pod.clone()),
            key(&cluster("prod"), pod),
            "another cluster, another filter"
        );
    }
}
