//! [`ClusterCatalog`]: the catalog entries and the user's marks on them.

use std::collections::HashMap;
use std::sync::Arc;

use futures::lock::Mutex;
use futures::stream::{BoxStream, StreamExt as _};
use jiff::Timestamp;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ports::{
    ClockPort, ClusterSourcePort, SourceId, SourcesChanged, StateKey, StatePort, StatePortExt as _,
    StateTable,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::entry::CatalogEntry;

/// The state table the catalog keeps its marks in, one row per [`ClusterId`].
pub const CATALOG_TABLE: &str = "cluster_catalog";

/// What is remembered about one cluster.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Marks {
    #[serde(default)]
    favourite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_used: Option<Timestamp>,
}

/// How many favourite changes a slow subscriber may fall behind by.
const FAVOURITE_UPDATES: usize = 64;

/// A cluster was marked or unmarked as a favourite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavouriteChanged {
    /// The cluster.
    pub cluster: ClusterId,
    /// Whether it is a favourite now.
    pub favourite: bool,
}

/// The subscriber fell behind and missed `missed` favourite changes: read the catalog again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FavouritesLagged {
    /// How many changes were dropped for this subscriber.
    pub missed: u64,
}

/// Reads and marks the cluster catalog. See the [module docs](super).
///
/// Cheap to clone; clones share the write lock that keeps two marks on one row from losing
/// each other.
#[derive(Clone)]
pub struct ClusterCatalog {
    source: Arc<dyn ClusterSourcePort>,
    state: Arc<dyn StatePort>,
    clock: Arc<dyn ClockPort>,
    /// Held across a row's read-modify-write, so a favourite toggle and a last-used stamp on
    /// the same cluster both land.
    write: Arc<Mutex<()>>,
    favourites: broadcast::Sender<FavouriteChanged>,
}

impl std::fmt::Debug for ClusterCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterCatalog").finish_non_exhaustive()
    }
}

impl ClusterCatalog {
    /// A catalog over `source`, keeping its marks in `state` and stamping last-used times with
    /// `clock`.
    pub fn new(
        source: Arc<dyn ClusterSourcePort>,
        state: Arc<dyn StatePort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            source,
            state,
            clock,
            write: Arc::default(),
            favourites: broadcast::channel(FAVOURITE_UPDATES).0,
        }
    }

    /// Every context of every source, with its marks, in the order the source lists them (sort
    /// with [`CatalogEntry::cmp_default`]).
    ///
    /// Reads kubeconfig files and the state db only; no cluster is contacted. The sources and
    /// the contexts are read side by side. A state db that fails is logged and the entries come
    /// back without marks.
    ///
    /// # Errors
    ///
    /// The cluster source's error when it cannot list its contexts. A failing source *list* is
    /// not an error: the entries then show their source id instead of its label.
    pub async fn load(&self) -> OxiResult<Vec<CatalogEntry>> {
        let (contexts, sources, marks) = futures::join!(
            self.source.contexts(),
            self.source.sources(),
            self.read_marks()
        );
        let contexts = contexts?;
        let sources: HashMap<SourceId, _> = match sources {
            Ok(sources) => sources.into_iter().map(|s| (s.id.clone(), s)).collect(),
            Err(error) => {
                tracing::warn!(%error, "the cluster source list could not be read");
                HashMap::new()
            }
        };
        let mut marks = marks;
        Ok(contexts
            .into_iter()
            .map(|context| {
                let marks = marks.remove(&context.cluster).unwrap_or_default();
                CatalogEntry {
                    source: sources.get(&context.source).cloned(),
                    favourite: marks.favourite,
                    last_used: marks.last_used,
                    context,
                }
            })
            .collect())
    }

    /// The catalog's changes from now on: the cluster source's diffs, unchanged. A view re-runs
    /// [`load`](Self::load) when one arrives.
    pub fn changes(&self) -> BoxStream<'static, SourcesChanged> {
        self.source.subscribe()
    }

    /// Re-reads the sources now. The resulting diff also reaches [`changes`](Self::changes).
    ///
    /// # Errors
    ///
    /// The cluster source's error.
    pub async fn reload(&self) -> OxiResult<SourcesChanged> {
        self.source.reload().await
    }

    /// Every favourite change made through this catalog (and its clones) from now on, so a view
    /// that shows favourites (the hotbar) follows a change made anywhere (the catalog's star, a
    /// command from the palette or an agent).
    pub fn favourite_changes(
        &self,
    ) -> BoxStream<'static, Result<FavouriteChanged, FavouritesLagged>> {
        futures::stream::unfold(self.favourites.subscribe(), |mut rx| async move {
            match rx.recv().await {
                Ok(change) => Some((Ok(change), rx)),
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    Some((Err(FavouritesLagged { missed }), rx))
                }
                Err(broadcast::error::RecvError::Closed) => None,
            }
        })
        .boxed()
    }

    /// Marks `cluster` as a favourite (`Some(true)`), clears it (`Some(false)`) or flips it
    /// (`None`). Returns the new value. Announces the change on
    /// [`favourite_changes`](Self::favourite_changes) once it is stored.
    ///
    /// # Errors
    ///
    /// The state port's error; nothing changed then.
    pub async fn set_favourite(
        &self,
        cluster: &ClusterId,
        favourite: Option<bool>,
    ) -> OxiResult<bool> {
        let _write = self.write.lock().await;
        let mut marks = self.read_row(cluster).await?;
        marks.favourite = favourite.unwrap_or(!marks.favourite);
        self.write_row(cluster, &marks).await?;
        // No subscriber is fine.
        let _ = self.favourites.send(FavouriteChanged {
            cluster: cluster.clone(),
            favourite: marks.favourite,
        });
        Ok(marks.favourite)
    }

    /// Stamps `cluster` as used now and returns the time.
    ///
    /// # Errors
    ///
    /// The state port's error; nothing changed then.
    pub async fn mark_used(&self, cluster: &ClusterId) -> OxiResult<Timestamp> {
        let now = self.clock.now();
        let _write = self.write.lock().await;
        let mut marks = self.read_row(cluster).await?;
        marks.last_used = Some(now);
        self.write_row(cluster, &marks).await?;
        Ok(now)
    }

    fn table() -> StateTable {
        StateTable::new(CATALOG_TABLE).expect("the catalog table name is valid")
    }

    async fn read_row(&self, cluster: &ClusterId) -> OxiResult<Marks> {
        let key = StateKey::new(cluster.as_str())?;
        match self.state.table_get_as::<Marks>(&Self::table(), &key).await {
            Ok(marks) => Ok(marks.unwrap_or_default()),
            // A row of an unexpected shape reads as "no marks": the write that follows replaces
            // it. Any other failure is real and must not be papered over with a default that
            // would then overwrite the stored marks.
            Err(error) if error.kind() == ErrorKind::Validation => {
                tracing::warn!(%error, %cluster, "an unreadable catalog row is treated as empty");
                Ok(Marks::default())
            }
            Err(error) => Err(error),
        }
    }

    async fn write_row(&self, cluster: &ClusterId, marks: &Marks) -> OxiResult<()> {
        let key = StateKey::new(cluster.as_str())?;
        self.state.table_put_as(&Self::table(), &key, marks).await
    }

    /// Every stored row, keyed by cluster. Failures and malformed rows are logged and skipped.
    async fn read_marks(&self) -> HashMap<ClusterId, Marks> {
        let rows = match self.state.table_list(&Self::table(), None).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "the catalog marks could not be read");
                return HashMap::new();
            }
        };
        rows.into_iter()
            .filter_map(|(key, value)| {
                let cluster = key.as_str().parse::<ClusterId>().ok()?;
                match serde_json::from_value::<Marks>(value) {
                    Ok(marks) => Some((cluster, marks)),
                    Err(error) => {
                        tracing::warn!(%error, %cluster, "a catalog row has an unexpected shape");
                        None
                    }
                }
            })
            .collect()
    }
}
