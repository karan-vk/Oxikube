//! [`ColumnPrefs`]: the user's column layout for one kind (order, visibility, widths, sort),
//! saved in the [`StatePort`] under `table.columns.<group>/<Kind>`.
//!
//! The layout is per kind, not per cluster: Pods look the same in every cluster, as in Lens.
//! Versions of a kind share it (`autoscaling/v1` and `v2`). Ids only, no object data: nothing
//! secret is stored (non-negotiable 5).
//!
//! Reads and writes are async and never run on the UI thread: the table loads its prefs on a
//! background task when it opens (defaults until they arrive) and saves through
//! [`PrefsWriter`], one background task per table that writes the newest snapshot in order.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, AppContext as _};
use oxikube_domain::ids::Gvk;
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ports::{StateKey, StatePort, StatePortExt as _};
use serde::{Deserialize, Serialize};

/// The version this build writes.
pub const PREFS_VERSION: u32 = 1;

/// The state key prefix of column layouts.
pub const PREFS_PREFIX: &str = "table.columns.";

/// A saved sort: the column and its direction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSort {
    /// The column id.
    pub column: String,
    /// Largest first.
    #[serde(default)]
    pub descending: bool,
}

/// The user's column layout of one kind. Every field is a choice the user made; anything not
/// in it follows the defaults (default columns shown, wide ones hidden, provider order).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ColumnPrefs {
    /// [`PREFS_VERSION`] of the build that wrote it.
    #[serde(default)]
    pub version: u32,
    /// Column ids in the user's order. Ids the kind no longer has are ignored; columns it does
    /// not name follow in the provider's order.
    #[serde(default)]
    pub order: Vec<String>,
    /// Columns the user showed (`true`) or hid (`false`), by id.
    #[serde(default)]
    pub visible: BTreeMap<String, bool>,
    /// Widths the user dragged to, in unscaled pixels, by id.
    #[serde(default)]
    pub widths: BTreeMap<String, f32>,
    /// The sort the user chose; `None` is the store's default order (namespace, name).
    #[serde(default)]
    pub sort: Option<SavedSort>,
}

/// The state key of `gvk`'s layout: `table.columns.<group>/<Kind>` (`core` for the core group).
///
/// # Errors
///
/// `Validation` when the kind's name makes an invalid state key.
pub fn prefs_key(gvk: &Gvk) -> OxiResult<StateKey> {
    kind_key(PREFS_PREFIX, gvk)
}

/// The per-kind state key `<prefix><group>/<Kind>` (`core` for the core group); versions of a
/// kind share it.
pub(crate) fn kind_key(prefix: &str, gvk: &Gvk) -> OxiResult<StateKey> {
    let group = if gvk.group.is_empty() {
        "core"
    } else {
        &gvk.group
    };
    StateKey::new(format!("{prefix}{group}/{}", gvk.kind))
}

/// Reads and writes one kind's [`ColumnPrefs`].
#[derive(Clone)]
pub struct ColumnPrefsStore {
    state: Arc<dyn StatePort>,
    key: StateKey,
}

impl ColumnPrefsStore {
    /// The store of `gvk`'s layout.
    ///
    /// # Errors
    ///
    /// `Validation` when the kind's name makes an invalid state key.
    pub fn new(state: Arc<dyn StatePort>, gvk: &Gvk) -> OxiResult<Self> {
        Ok(Self {
            state,
            key: prefs_key(gvk)?,
        })
    }

    /// The saved layout, `None` when there is none. A row of another shape or a newer version
    /// reads as `None` (the next save replaces it): a bad row must never break the table.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn load(&self) -> OxiResult<Option<ColumnPrefs>> {
        match self.state.kv_get_as::<ColumnPrefs>(&self.key).await {
            Ok(Some(prefs)) if prefs.version <= PREFS_VERSION => Ok(Some(prefs)),
            Ok(Some(_)) => {
                tracing::warn!(key = %self.key.as_str(), "column layout from a newer build: ignored");
                Ok(None)
            }
            Ok(None) => Ok(None),
            Err(error) if error.kind() == ErrorKind::Validation => {
                tracing::warn!(%error, "saved column layout is unreadable: ignored");
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Writes `prefs`, replacing the saved layout.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn save(&self, prefs: &ColumnPrefs) -> OxiResult<()> {
        let mut row = prefs.clone();
        row.version = PREFS_VERSION;
        self.state.kv_set_as(&self.key, &row).await
    }
}

impl std::fmt::Debug for ColumnPrefsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColumnPrefsStore")
            .field("key", &self.key.as_str())
            .finish_non_exhaustive()
    }
}

/// Sends layout snapshots to the one background task that saves them, in order; a burst (a
/// column dragged wider) collapses into one write of the newest. The task is detached on
/// purpose: it ends when the table (the sender) goes, after writing what was queued, so a
/// change made right before closing is kept. It holds no entity.
pub struct PrefsWriter {
    tx: UnboundedSender<ColumnPrefs>,
}

impl PrefsWriter {
    /// Starts the writer task for `store`.
    pub fn spawn(store: ColumnPrefsStore, cx: &App) -> Self {
        let (tx, mut rx) = unbounded::<ColumnPrefs>();
        cx.background_spawn(async move {
            while let Some(mut prefs) = rx.next().await {
                while let Ok(newer) = rx.try_recv() {
                    prefs = newer;
                }
                if let Err(error) = store.save(&prefs).await {
                    tracing::warn!(%error, "saving the column layout failed");
                }
            }
        })
        .detach();
        Self { tx }
    }

    /// Queues `prefs` to be written.
    pub fn save(&self, prefs: ColumnPrefs) {
        // The task outlives every sender, so this only fails once it is gone.
        self.tx.unbounded_send(prefs).ok();
    }
}
