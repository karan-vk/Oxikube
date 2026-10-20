//! [`FavouriteStore`]: the favourites in the state store's `port_forward_favourites` table.

use std::sync::Arc;

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{StateKey, StatePort, StatePortExt as _, StateTable};

use super::model::{FavouriteForward, FavouriteId};

/// The state table the favourites live in: one row per favourite, keyed by its id.
pub const TABLE: &str = "port_forward_favourites";

/// Favourites over [`StatePort`] (SQLite in the app, in memory in tests).
#[derive(Clone)]
pub struct FavouriteStore {
    state: Arc<dyn StatePort>,
    table: StateTable,
}

impl FavouriteStore {
    /// A store over `state`.
    pub fn new(state: Arc<dyn StatePort>) -> Self {
        Self {
            state,
            table: StateTable::new(TABLE).expect("a valid table name"),
        }
    }

    fn key(id: &FavouriteId) -> OxiResult<StateKey> {
        StateKey::new(id.as_str())
    }

    /// Every favourite, by id. A row of an unknown shape (a newer version wrote it) is skipped
    /// and logged once, never an error: one bad row must not hide the rest.
    ///
    /// # Errors
    ///
    /// The state store's own errors.
    pub async fn list(&self) -> OxiResult<Vec<FavouriteForward>> {
        let rows = self.state.table_list(&self.table, None).await?;
        Ok(rows
            .into_iter()
            .filter_map(|(key, value)| match serde_json::from_value(value) {
                Ok(favourite) => Some(favourite),
                Err(error) => {
                    tracing::warn!(key = key.as_str(), %error, "skipping an unreadable port-forward favourite");
                    None
                }
            })
            .collect())
    }

    /// The favourites of `cluster`.
    ///
    /// # Errors
    ///
    /// The state store's own errors.
    pub async fn list_for(&self, cluster: &ClusterId) -> OxiResult<Vec<FavouriteForward>> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .filter(|f| f.cluster() == cluster)
            .collect())
    }

    /// The favourite `id`, if saved.
    ///
    /// # Errors
    ///
    /// The state store's own errors.
    pub async fn get(&self, id: &FavouriteId) -> OxiResult<Option<FavouriteForward>> {
        match self.state.table_get_as(&self.table, &Self::key(id)?).await {
            Ok(found) => Ok(found),
            // A row of an unknown shape reads as absent, like in `list`.
            Err(e) if e.kind() == oxikube_domain::ErrorKind::Validation => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Saves `favourite`, replacing the one with its id.
    ///
    /// # Errors
    ///
    /// The state store's own errors.
    pub async fn put(&self, favourite: &FavouriteForward) -> OxiResult<()> {
        self.state
            .table_put_as(&self.table, &Self::key(&favourite.id)?, favourite)
            .await
    }

    /// Removes the favourite `id`. Returns whether it was saved.
    ///
    /// # Errors
    ///
    /// The state store's own errors.
    pub async fn remove(&self, id: &FavouriteId) -> OxiResult<bool> {
        self.state.table_delete(&self.table, &Self::key(id)?).await
    }
}
