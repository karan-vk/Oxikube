//! A namespace change shown in the frame after the input (E05-P600).
//!
//! When the session's namespace selection changes, the subscription is rescoped (`feed`) and the
//! store reseeds it off the UI thread: the new rows come as a snapshot a frame or more later.
//! Until then the table shows what it can now, from the rows it already holds:
//!
//! - **Narrowing** (all namespaces to one, or a set to a subset): the held rows of the namespaces
//!   that stay are exactly the new list, so they are kept and the others dropped, in the same
//!   update. The reseed then reconciles them (the store had them already).
//! - **Widening or moving** (one namespace to another, or to all): the held rows of the namespaces
//!   that stay are kept, and the table is marked as loading (the stale badge over rows, the
//!   loading state without) until the snapshot brings the rest.
//!
//! The rescope leaves the subscription with a snapshot to deliver next, so no row operation is
//! ever applied to the narrowed rows.
//!
//! The change reaches the table twice: at once through the session echo, when the command ran on
//! the UI thread ([`oxikube_workspace::cluster::observe_session_echo`]), and later through the
//! session's update stream. Only the first finds a new scope; the second compares and stops.

use std::sync::Arc;

use gpui::{Context, Subscription};
use oxikube_app::SessionChange;
use oxikube_app::store::{FeedState, StoreObject};
use oxikube_domain::session::WatchScope;
use oxikube_workspace::cluster::{EchoItem, observe_session_echo};

use super::view::ResourceTable;

impl ResourceTable {
    /// Follows the session echo: a namespace change of this table's cluster made on the UI thread
    /// rescopes the table in that update.
    pub(super) fn follow_session_echo(cx: &mut Context<Self>) -> Subscription {
        observe_session_echo(cx, |view: &mut Self, items: &[EchoItem], cx| {
            let changed = items.iter().any(|item| match item {
                Ok(update) => {
                    update.cluster == view.cluster
                        && matches!(update.change, SessionChange::NamespaceChanged(_))
                }
                // Missed some: re-read, as the stream's follower does.
                Err(_) => true,
            });
            if changed {
                // As the stream's follower does: the scope, then the columns (one namespace hides
                // the Namespace column), so both change in this frame and not in two.
                view.resubscribe(cx);
                view.refresh_columns(cx);
            }
        })
    }

    /// Shows the rows already held for `scope` (the subscription was just moved there from
    /// `before`) and redraws in this update. See the [module docs](self).
    pub(super) fn show_held_rows(
        &mut self,
        before: &WatchScope,
        scope: &WatchScope,
        cx: &mut Context<Self>,
    ) {
        let complete = covers(before, scope);
        let selection_before = self.table.read(cx, |d| d.selection.len());
        let selection_after = self.table.update_quiet(cx, |d| {
            if let WatchScope::Namespaces(names) = scope {
                let kept: Vec<Arc<StoreObject>> = d
                    .rows
                    .iter()
                    .filter(|row| in_scope(row, names))
                    .cloned()
                    .collect();
                d.selection.apply_snapshot(&mut d.rows, &kept);
            }
            if !complete {
                d.state = FeedState::Warming;
            }
            d.selection.len()
        });
        if selection_after != selection_before {
            self.selection_changed(cx);
        }
        // Direct, not coalesced: this is the input's frame.
        cx.notify();
    }
}

/// Whether `names` (sorted, as [`WatchScope::Namespaces`] keeps them) lists `row`'s namespace.
fn in_scope(row: &StoreObject, names: &[String]) -> bool {
    row.namespace()
        .is_some_and(|ns| names.binary_search_by(|name| name.as_str().cmp(ns)).is_ok())
}

/// Whether every object of `scope` was in `before`: then the rows held for `before` hold all of
/// `scope`'s.
pub(super) fn covers(before: &WatchScope, scope: &WatchScope) -> bool {
    match (before, scope) {
        (WatchScope::Cluster, _) => true,
        (WatchScope::Namespaces(_), WatchScope::Cluster) => false,
        (WatchScope::Namespaces(held), WatchScope::Namespaces(wanted)) => {
            wanted.iter().all(|name| held.binary_search(name).is_ok())
        }
    }
}
