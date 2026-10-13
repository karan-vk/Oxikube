//! The events view's filters saved per cluster, in the [`StatePort`] under
//! `events.filter.<cluster>` (epic E14 "done when" 2: filters persist per cluster).
//!
//! The row holds what the user typed in the toolbar and the faults toggle, nothing from the
//! cluster. Reads and writes are async and never run on the UI thread: the view loads the row
//! when it opens and hands every change to one background writer that keeps only the newest of
//! a burst, so typing a filter costs one write.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, AppContext as _};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ports::{StateKey, StatePort, StatePortExt as _};
use serde::{Deserialize, Serialize};

use crate::filter::EventsFilters;

/// The version this build writes.
pub const FILTERS_VERSION: u32 = 1;

/// The state key prefix of the saved filters.
pub const FILTERS_PREFIX: &str = "events.filter.";

#[derive(Serialize, Deserialize)]
struct Row {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    filters: EventsFilters,
}

/// The state key of `cluster`'s events filters: `events.filter.<cluster>`.
///
/// # Errors
///
/// `Validation` when the cluster id makes an invalid key.
pub fn filters_key(cluster: &ClusterId) -> OxiResult<StateKey> {
    StateKey::new(format!("{FILTERS_PREFIX}{cluster}"))
}

/// Reads and writes one cluster's saved events filters.
#[derive(Clone)]
pub struct SavedFilters {
    state: Arc<dyn StatePort>,
    key: StateKey,
}

impl std::fmt::Debug for SavedFilters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedFilters")
            .field("key", &self.key.as_str())
            .finish_non_exhaustive()
    }
}

impl SavedFilters {
    /// The saved filters of `cluster`.
    ///
    /// # Errors
    ///
    /// `Validation` when the cluster id makes an invalid key.
    pub fn new(state: Arc<dyn StatePort>, cluster: &ClusterId) -> OxiResult<Self> {
        Ok(Self {
            state,
            key: filters_key(cluster)?,
        })
    }

    /// The saved filters; `None` without a row. A row of another shape or a newer version reads
    /// as `None`: a bad row must never break the view.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn load(&self) -> OxiResult<Option<EventsFilters>> {
        match self.state.kv_get_as::<Row>(&self.key).await {
            Ok(Some(row)) if row.version <= FILTERS_VERSION => Ok(Some(row.filters)),
            Ok(_) => Ok(None),
            Err(error) if error.kind() == ErrorKind::Validation => {
                tracing::warn!(%error, "saved events filters are unreadable: ignored");
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Writes `filters`; the defaults remove the row.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn save(&self, filters: &EventsFilters) -> OxiResult<()> {
        if !filters.is_active() {
            return self.state.kv_delete(&self.key).await.map(drop);
        }
        let row = Row {
            version: FILTERS_VERSION,
            filters: filters.clone(),
        };
        self.state.kv_set_as(&self.key, &row).await
    }
}

/// Sends filters to the one background task that saves them, in order; a burst collapses into
/// one write of the newest. The task ends when the view (the sender) goes, after writing what
/// was queued. It holds no entity.
pub struct FiltersWriter {
    tx: UnboundedSender<EventsFilters>,
}

impl FiltersWriter {
    /// Starts the writer task for `saved`.
    pub fn spawn(saved: SavedFilters, cx: &App) -> Self {
        let (tx, mut rx) = unbounded::<EventsFilters>();
        cx.background_spawn(async move {
            while let Some(mut filters) = rx.next().await {
                while let Ok(newer) = rx.try_recv() {
                    filters = newer;
                }
                if let Err(error) = saved.save(&filters).await {
                    tracing::warn!(%error, "saving the events filters failed");
                }
            }
        })
        .detach();
        Self { tx }
    }

    /// Queues `filters` to be written.
    pub fn save(&self, filters: EventsFilters) {
        // The task outlives every sender, so this only fails once it is gone.
        self.tx.unbounded_send(filters).ok();
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use oxikube_domain::ids::ContextName;
    use oxikube_testkit::FakeStatePort;

    use super::*;

    fn cluster(name: &str) -> ClusterId {
        ClusterId::new("tests", &ContextName::from(name))
    }

    #[test]
    fn filters_are_saved_per_cluster_and_the_defaults_remove_the_row() {
        let state: Arc<dyn StatePort> = Arc::new(FakeStatePort::new());
        let dev = SavedFilters::new(state.clone(), &cluster("dev")).unwrap();
        let prod = SavedFilters::new(state.clone(), &cluster("prod")).unwrap();
        let faults = EventsFilters {
            faults_only: true,
            text: "image".into(),
            ..EventsFilters::default()
        };
        block_on(dev.save(&faults)).unwrap();
        assert_eq!(block_on(dev.load()).unwrap(), Some(faults));
        assert_eq!(block_on(prod.load()).unwrap(), None, "another cluster");
        block_on(dev.save(&EventsFilters::default())).unwrap();
        assert_eq!(block_on(dev.load()).unwrap(), None);
        assert!(
            filters_key(&cluster("dev"))
                .unwrap()
                .as_str()
                .starts_with(FILTERS_PREFIX)
        );
    }

    #[test]
    fn a_row_from_a_newer_build_or_of_another_shape_reads_as_none() {
        let state: Arc<dyn StatePort> = Arc::new(FakeStatePort::new());
        let saved = SavedFilters::new(state.clone(), &cluster("dev")).unwrap();
        let key = filters_key(&cluster("dev")).unwrap();
        block_on(state.kv_set(&key, serde_json::json!({ "version": 99 }))).unwrap();
        assert_eq!(block_on(saved.load()).unwrap(), None);
        block_on(state.kv_set(&key, serde_json::json!("garbage"))).unwrap();
        assert_eq!(block_on(saved.load()).unwrap(), None);
    }
}
