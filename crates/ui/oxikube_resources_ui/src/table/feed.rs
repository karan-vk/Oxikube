//! The table's rows: the [`ResourceStore`] subscription, applied delta by delta.
//!
//! The store keeps the rows sorted and filtered (E07-S01): the table asks for its order with a
//! [`SortKey`] and never sorts itself. The subscription is a stream polled by one foreground
//! task the view owns (replaced, never cleared from inside, when the view re-subscribes). Each
//! wake drains everything pending (the store hands over one coalesced delta per poll) and
//! applies it in one update, then redraws through `notify_coalesced`, so a burst of feed events
//! costs at most one redraw per frame.
//!
//! The view follows its session: a namespace change rescopes the subscription (shared feeds,
//! rows kept for namespaces that stay), a reconnect (a new store) re-subscribes, a capability
//! change re-reads the columns.

use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use futures::{Stream as _, StreamExt as _};
use gpui::{AsyncApp, Context, Task, WeakEntity};
use jiff::Timestamp;
use oxikube_app::store::{
    CellSortKey, FeedKind, RowChange, SortField, SortKey, StoreDelta, StoreQuery,
};
use oxikube_app::{ColumnProvider, TableColumns};
use oxikube_domain::ids::ClusterId;
use oxikube_runtime::notify_coalesced;

use super::states::{poll_warnings, scope_label};
use super::view::{ResourceTable, ResourceTableDeps};

impl ResourceTable {
    /// The sort the user chose: the layout's column through the provider's typed cell keys.
    pub(super) fn chosen_sort(&self, cx: &gpui::App) -> Option<SortKey> {
        self.table.read(cx, |d| {
            d.layout.sort().map(|(column, descending)| SortKey {
                field: SortField::Cell(CellSortKey::new(column.clone(), d.provider.clone())),
                descending: *descending,
            })
        })
    }

    /// The sort the store keeps the rows in: the chosen column, else best match first while a
    /// fuzzy filter is on, else the store's default (namespace, name).
    pub(super) fn sort_key(&self, cx: &gpui::App) -> SortKey {
        self.filter_parts.sort(self.chosen_sort(cx))
    }

    /// Hands the layout's sort to the subscription (the store re-sorts off the UI thread and
    /// sends a snapshot).
    pub(super) fn apply_sort(&mut self, cx: &mut Context<Self>) {
        let key = self.sort_key(cx);
        if let Some(subscription) = &mut self.subscription {
            subscription.set_sort(key);
        }
    }

    /// Subscribes to the session's store, or moves the subscription to the session's current
    /// scope. Does nothing while the session is not connected (the tab shows the connect view;
    /// the rows stay until it reconnects).
    pub(super) fn resubscribe(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.deps.sessions.get(&self.cluster) else {
            return;
        };
        if !session.is_connected() {
            return;
        }
        let Some(store) = self.deps.stores.for_session(&session) else {
            return;
        };
        let scope = session.watch_scope(self.kind.scope());
        let scope_text: Arc<str> = scope_label(&scope, self.kind.namespaced).into();
        self.table.update_quiet(cx, |d| d.labels.scope = scope_text);
        let same_store = self
            .store
            .as_ref()
            .is_some_and(|current| current.is_same(&store));
        if same_store && let Some(subscription) = &mut self.subscription {
            if subscription.query().scope != scope {
                subscription.rescope(scope);
            }
            return;
        }
        let query = StoreQuery::new(self.kind.gvk.clone(), scope)
            .with_filter(self.filter_parts.filter.clone())
            .with_selector(self.filter_parts.selector.clone())
            .with_sort(self.sort_key(cx));
        let plan = store.plan(&self.kind.gvk).kind;
        // Before the feed opens: the server's warnings are not replayed.
        self.warning_task = Some(poll_warnings(store.warnings(), cx));
        self.subscription = Some(store.subscribe(query));
        self.store = Some(store);
        if self.feed_kind != Some(plan) {
            self.feed_kind = Some(plan);
            if plan != FeedKind::Table {
                let core: Arc<dyn ColumnProvider> = self.deps.columns.clone();
                let current = self.table.read(cx, |d| d.provider.clone());
                if !std::ptr::addr_eq(Arc::as_ptr(&current), Arc::as_ptr(&core)) {
                    self.set_provider(core, cx);
                }
            }
        }
        self.feed_task = Some(Self::poll_feed(cx));
    }

    /// The task that polls the subscription until the view goes.
    fn poll_feed(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            futures::future::poll_fn(|task_cx| {
                match this.update(cx, |view, cx| view.drain(task_cx, cx)) {
                    Ok(true) => Poll::Pending,
                    _ => Poll::Ready(()),
                }
            })
            .await;
        })
    }

    /// Applies every delta the subscription has ready. Returns whether to keep polling.
    fn drain(&mut self, task_cx: &mut std::task::Context<'_>, cx: &mut Context<Self>) -> bool {
        let Some(subscription) = self.subscription.as_mut() else {
            return false;
        };
        let mut deltas = Vec::new();
        let live = loop {
            match Pin::new(&mut *subscription).poll_next(task_cx) {
                Poll::Ready(Some(delta)) => deltas.push(delta),
                Poll::Ready(None) => break false,
                Poll::Pending => break true,
            }
        };
        if !deltas.is_empty() {
            for delta in deltas {
                self.apply(delta, cx);
            }
            notify_coalesced(cx);
        }
        live
    }

    /// Applies one delta: columns first (a Table feed's provider), then the rows, keeping the
    /// selection by identity.
    pub(super) fn apply(&mut self, delta: StoreDelta, cx: &mut Context<Self>) {
        if let Some(columns) = &delta.columns {
            let provider: Arc<dyn ColumnProvider> = Arc::new(TableColumns::new(
                &columns.columns,
                columns.source,
                self.kind.scope(),
            ));
            self.set_provider(provider, cx);
        }
        let StoreDelta {
            rows,
            state,
            len,
            total,
            ..
        } = delta;
        self.filter
            .update(cx, |bar, cx| bar.set_counts(len, total, cx));
        let selection_before = self.table.read(cx, |d| d.selection.len());
        let selection_after = self.table.update_quiet(cx, |d| {
            d.state = state;
            d.now = Timestamp::now();
            match &rows {
                RowChange::Snapshot(all) => d.selection.apply_snapshot(&mut d.rows, all),
                RowChange::Ops(ops) => d.selection.apply_ops(&mut d.rows, ops),
                RowChange::Unchanged => {}
            }
            d.selection.len()
        });
        if selection_after != selection_before {
            self.selection_changed(cx);
        }
    }

    /// The task that follows the cluster's session updates.
    pub(super) fn follow_session(
        cluster: &ClusterId,
        deps: &ResourceTableDeps,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let mut updates = deps.sessions.subscribe();
        let cluster = cluster.clone();
        cx.spawn(async move |this, cx| {
            while let Some(update) = updates.next().await {
                // A lagged subscriber re-reads the session, like any update of ours.
                if update.as_ref().is_ok_and(|u| u.cluster != cluster) {
                    continue;
                }
                let followed = this.update(cx, |view, cx| {
                    view.resubscribe(cx);
                    view.refresh_columns(cx);
                });
                if followed.is_err() {
                    break;
                }
            }
        })
    }

    /// Re-reads the provider's columns (the session's capabilities may have changed: metrics
    /// columns come and go), keeping the layout.
    fn refresh_columns(&mut self, cx: &mut Context<Self>) {
        self.relayout(None, cx);
    }
}
