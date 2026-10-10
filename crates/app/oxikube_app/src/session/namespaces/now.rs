//! Selecting now and remembering later: [`NamespaceService::select_now`] (E05-P600).
//!
//! Setting the session's selection is in memory (one lock, one `NamespaceChanged`), so the UI does
//! it in the update that took the input and the table narrows in that frame. Remembering it is a
//! `StatePort` write, so it comes back as [`SelectedNow::remember`] for the caller to run off the
//! UI thread. Every selection path (`select`, the debounce, a prune) goes through
//! [`apply_now`](NamespaceService::apply_now), which numbers the selections it applies: the
//! remembered prefs take the last one applied to the session, whatever order the writes finish
//! in, so the remembered selection is the one the session shows.

use futures::future::BoxFuture;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;

use super::service::{NamespaceOutcome, NamespaceService};

/// A selection applied to the session ([`NamespaceService::select_now`]); remembering it is
/// still to run.
#[must_use = "the selection is not remembered until `remember` runs"]
pub struct SelectedNow {
    /// Whether the session's selection changed (a `NamespaceChanged` was sent).
    pub session_changed: bool,
    remember: BoxFuture<'static, OxiResult<NamespaceOutcome>>,
}

impl std::fmt::Debug for SelectedNow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectedNow")
            .field("session_changed", &self.session_changed)
            .finish_non_exhaustive()
    }
}

impl SelectedNow {
    /// Remembers the selection in `StatePort` (unless a later selection was applied meanwhile,
    /// which is remembered instead). Run it off the UI thread.
    ///
    /// # Errors
    ///
    /// The state store's error.
    pub fn remember(self) -> BoxFuture<'static, OxiResult<NamespaceOutcome>> {
        self.remember
    }
}

impl NamespaceService {
    /// Sets the selection on the session now, on this thread (in memory: one
    /// `NamespaceChanged` when it differs), and cancels a pending
    /// [`select_debounced`](Self::select_debounced). Remembering it is returned, to run off the
    /// UI thread. [`select`](Self::select) is this followed by the remembering.
    ///
    /// # Errors
    ///
    /// `NotFound` when the session is not open.
    pub fn select_now(
        &self,
        cluster: &ClusterId,
        selection: NamespaceSelection,
    ) -> OxiResult<SelectedNow> {
        self.next_ticket(cluster);
        self.apply_now(cluster, selection)
    }

    /// Sets `selection` on the session and numbers it; the returned future remembers it if it
    /// is still the last one applied when its turn comes.
    pub(super) fn apply_now(
        &self,
        cluster: &ClusterId,
        selection: NamespaceSelection,
    ) -> OxiResult<SelectedNow> {
        // Numbered under the same lock as the session change, so the numbers follow the
        // order the session saw.
        let (session_changed, applied) = {
            let mut applied = self.shared.applied.lock();
            let changed = self
                .shared
                .manager
                .set_namespace_selection(cluster, selection.clone())?;
            let number = applied.entry(cluster.clone()).or_insert(0);
            *number += 1;
            (changed, *number)
        };
        let this = self.clone();
        let cluster = cluster.clone();
        let remember = Box::pin(async move {
            this.ensure_loaded(&cluster).await?;
            // An explicit choice is remembered even when it equals the selection the session
            // opened with (the cache is seeded from it), so it outlives a changed default.
            let unremembered = !this.shared.stored.lock().contains(&cluster);
            let mut outcome = this
                .mutate_inner(&cluster, unremembered, |prefs| {
                    // A selection applied after this one owns the prefs: leave them to it.
                    if this.shared.applied.lock().get(&cluster) != Some(&applied) {
                        return false;
                    }
                    let changed = prefs.selection != selection;
                    prefs.selection = selection;
                    changed
                })
                .await?;
            outcome.changed |= session_changed;
            Ok(outcome)
        });
        Ok(SelectedNow {
            session_changed,
            remember,
        })
    }
}
