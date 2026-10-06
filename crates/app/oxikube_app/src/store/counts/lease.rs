//! [`CountsLease`]: the feeds a view needs open so that its counts are there.

use oxikube_domain::session::{NamespaceSelection, WatchScope};

use super::{CountState, CountTarget};
use crate::store::query::{StoreFilter, StoreQuery};
use crate::store::service::ResourceStore;
use crate::store::subscription::Subscription;

/// Keeps one feed per target open (through the normal subscribe path, so the watch budget
/// decides) without building a row list: each subscription's filter matches no object, so the
/// store's per-subscriber index stays empty and costs nothing per event.
///
/// Dropping the lease releases its feeds; the store's grace timer stops the ones nobody else
/// uses. Read the numbers with [`counts`](Self::counts).
pub struct CountsLease {
    store: ResourceStore,
    targets: Vec<CountTarget>,
    selection: NamespaceSelection,
    subs: Vec<Subscription>,
}

impl std::fmt::Debug for CountsLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CountsLease")
            .field("targets", &self.targets.len())
            .finish_non_exhaustive()
    }
}

/// A filter no object passes: names are never empty.
fn nothing() -> StoreFilter {
    StoreFilter {
        name: Some(String::new()),
        ..StoreFilter::default()
    }
}

impl ResourceStore {
    /// Opens (or joins) the feeds of `targets` under `selection` and keeps them for as long as
    /// the returned lease lives. Never blocks: feeds start on the store's spawner.
    pub fn lease_counts(
        &self,
        targets: Vec<CountTarget>,
        selection: &NamespaceSelection,
    ) -> CountsLease {
        let subs = targets
            .iter()
            .map(|t| self.subscribe(query(t, selection)))
            .collect();
        CountsLease {
            store: self.clone(),
            targets,
            selection: selection.clone(),
            subs,
        }
    }
}

fn query(target: &CountTarget, selection: &NamespaceSelection) -> StoreQuery {
    StoreQuery::new(
        target.gvk.clone(),
        WatchScope::derive(selection, target.scope),
    )
    .with_filter(nothing())
}

impl CountsLease {
    /// The kinds this lease keeps open.
    pub fn targets(&self) -> &[CountTarget] {
        &self.targets
    }

    /// The count of each target, in order.
    pub fn counts(&self) -> Vec<CountState> {
        self.store.counts(&self.targets, &self.selection)
    }

    /// Follows a namespace change: keeps the feeds of namespaces that stay and swaps the rest
    /// ([`Subscription::rescope`]).
    pub fn rescope(&mut self, selection: &NamespaceSelection) {
        if &self.selection == selection {
            return;
        }
        self.selection = selection.clone();
        for (sub, target) in self.subs.iter_mut().zip(&self.targets) {
            sub.rescope(WatchScope::derive(selection, target.scope));
        }
    }
}
