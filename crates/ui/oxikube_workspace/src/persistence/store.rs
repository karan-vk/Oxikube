//! [`LayoutStore`]: one window's saved layout in the [`StatePort`].

use std::sync::Arc;

use oxikube_domain::OxiResult;
use oxikube_ports::{StateKey, StatePort, StateTable};

use super::model::{LAYOUT_TABLE, LayoutError, SerializedWorkspace};

/// What reading a saved layout found.
#[derive(Debug)]
pub enum LoadOutcome {
    /// Nothing saved yet (first launch, or the state was reset).
    Missing,
    /// A usable layout.
    Loaded(SerializedWorkspace),
    /// Something is stored but cannot be used (newer build, damaged JSON). The window starts with
    /// its default layout; the next save replaces the stored row.
    Discarded(LayoutError),
}

/// Reads and writes the layout of one window through a [`StatePort`] (SQLite in the app, the fake
/// in tests). Every call is async: the port runs it off the UI thread.
#[derive(Clone)]
pub struct LayoutStore {
    state: Arc<dyn StatePort>,
    table: StateTable,
    key: StateKey,
}

impl LayoutStore {
    /// A store for the window `window_id` (`[A-Za-z0-9_.:/-]`, at most 256 bytes; the main
    /// window is [`MAIN_WINDOW_ID`](super::MAIN_WINDOW_ID)).
    ///
    /// # Errors
    ///
    /// `Validation` when `window_id` is not a valid state key.
    pub fn new(state: Arc<dyn StatePort>, window_id: &str) -> OxiResult<Self> {
        Ok(Self {
            state,
            table: StateTable::new(LAYOUT_TABLE)?,
            key: StateKey::new(window_id)?,
        })
    }

    /// Reads the saved layout. A stored value that cannot be read is
    /// [`LoadOutcome::Discarded`], not an error: a bad layout must never stop the app starting.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn load(&self) -> OxiResult<LoadOutcome> {
        let Some(value) = self.state.table_get(&self.table, &self.key).await? else {
            return Ok(LoadOutcome::Missing);
        };
        match SerializedWorkspace::from_json(value) {
            Ok(layout) => Ok(LoadOutcome::Loaded(layout)),
            Err(error) => {
                tracing::warn!(%error, window = self.key.as_str(), "saved layout discarded");
                Ok(LoadOutcome::Discarded(error))
            }
        }
    }

    /// Writes `layout`, replacing the previous one.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn save(&self, layout: &SerializedWorkspace) -> OxiResult<()> {
        self.state
            .table_put(&self.table, &self.key, layout.to_json())
            .await
    }

    /// Forgets the saved layout. Returns whether there was one.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn clear(&self) -> OxiResult<bool> {
        self.state.table_delete(&self.table, &self.key).await
    }
}

impl std::fmt::Debug for LayoutStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayoutStore")
            .field("window", &self.key.as_str())
            .finish_non_exhaustive()
    }
}
