//! [`HotbarStore`]: the order the user gave the hotbar, in the [`StatePort`].
//!
//! One row per window in the state table [`HOTBAR_TABLE`]: `{ "order": [cluster ids] }`.
//! Cluster ids only (non-negotiable 5).

use std::sync::Arc;

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{StateKey, StatePort, StatePortExt as _, StateTable};
use serde::{Deserialize, Serialize};

/// The state table the hotbar order is stored in, one row per window.
pub const HOTBAR_TABLE: &str = "hotbar";

/// What is stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Row {
    #[serde(default)]
    order: Vec<ClusterId>,
}

/// Reads and writes one window's hotbar order. Every call is async: the port runs it off the UI
/// thread.
#[derive(Clone)]
pub struct HotbarStore {
    state: Arc<dyn StatePort>,
    table: StateTable,
    key: StateKey,
}

impl HotbarStore {
    /// A store for the window `window_id` (`main` for the main window).
    ///
    /// # Errors
    ///
    /// `Validation` when `window_id` is not a valid state key.
    pub fn new(state: Arc<dyn StatePort>, window_id: &str) -> OxiResult<Self> {
        Ok(Self {
            state,
            table: StateTable::new(HOTBAR_TABLE)?,
            key: StateKey::new(window_id)?,
        })
    }

    /// The saved order, empty when nothing was saved or the row is unreadable (a bad row must
    /// never stop the hotbar).
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn load(&self) -> OxiResult<Vec<ClusterId>> {
        match self.state.table_get_as::<Row>(&self.table, &self.key).await {
            Ok(row) => Ok(row.unwrap_or_default().order),
            Err(error) if error.kind() == oxikube_domain::ErrorKind::Validation => {
                tracing::warn!(%error, "the saved hotbar order is unreadable: ignored");
                Ok(Vec::new())
            }
            Err(error) => Err(error),
        }
    }

    /// Writes `order`, replacing the previous one.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn save(&self, order: &[ClusterId]) -> OxiResult<()> {
        let row = Row {
            order: order.to_vec(),
        };
        self.state.table_put_as(&self.table, &self.key, &row).await
    }
}

impl std::fmt::Debug for HotbarStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotbarStore")
            .field("window", &self.key.as_str())
            .finish_non_exhaustive()
    }
}
