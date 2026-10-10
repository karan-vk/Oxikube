//! [`AliasRegistry`]: one [`AliasTable`] per cluster, sharing the user's aliases.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::AliasTarget;
use oxikube_domain::ids::ClusterId;
use parking_lot::Mutex;
use tokio::runtime::Handle;

use super::entry::{AliasConflict, AliasSource};
use super::follow::{self, AliasFollow};
use super::table::AliasTable;
use crate::session::ClusterSessionManager;

/// The alias tables of every cluster the app has seen. Discovery is per cluster (a CRD of one is
/// not an alias of another); the user's `aliases.json` is the same for all.
///
/// Cheap to clone; clones share the tables.
#[derive(Clone, Default)]
pub struct AliasRegistry {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    tables: Mutex<HashMap<ClusterId, AliasTable>>,
    user: Mutex<Vec<(String, AliasTarget)>>,
}

impl std::fmt::Debug for AliasRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AliasRegistry")
            .field("clusters", &self.inner.tables.lock().len())
            .finish_non_exhaustive()
    }
}

impl AliasRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The table of `cluster`, created with the built-in and the user's aliases on first use.
    pub fn table(&self, cluster: &ClusterId) -> AliasTable {
        let mut tables = self.inner.tables.lock();
        if let Some(table) = tables.get(cluster) {
            return table.clone();
        }
        let table = AliasTable::new();
        let user = self.inner.user.lock().clone();
        if !user.is_empty() {
            table.set_user_aliases(user);
        }
        tables.insert(cluster.clone(), table.clone());
        table
    }

    /// Forgets the table of `cluster` (its session was closed). Holders of the table keep it.
    pub fn forget(&self, cluster: &ClusterId) {
        self.inner.tables.lock().remove(cluster);
    }

    /// Replaces the user's aliases in every table, present and future. The binary calls this
    /// with what `aliases.json` parsed to, at start-up and on every hot reload.
    ///
    /// Returns the user's aliases that hide a built-in one (`po` pointing somewhere else), so
    /// the caller can say so. Collisions with a cluster's discovery are cluster-specific and are
    /// in that cluster's [`AliasTable::conflicts`].
    pub fn set_user_aliases(&self, aliases: Vec<(String, AliasTarget)>) -> Vec<AliasConflict> {
        let probe = AliasTable::new();
        probe.set_user_aliases(aliases.clone());
        let shadowing = probe
            .conflicts()
            .into_iter()
            .filter(|c| c.winner.source == AliasSource::User)
            .collect();

        *self.inner.user.lock() = aliases.clone();
        let tables: Vec<AliasTable> = self.inner.tables.lock().values().cloned().collect();
        for table in tables {
            table.set_user_aliases(aliases.clone());
        }
        shadowing
    }

    /// Starts keeping the tables in step with `sessions`: a cluster that connects gets its
    /// served types, a CRD change updates only the groups it touches, a cluster that
    /// disconnects loses its discovered aliases. Work runs on `runtime`; each cluster has its
    /// own task, so a slow one delays no other. Dropping the handle stops it.
    pub fn follow(&self, sessions: &ClusterSessionManager, runtime: &Handle) -> AliasFollow {
        follow::start(self.clone(), sessions.clone(), runtime.clone())
    }
}
