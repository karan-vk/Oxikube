//! Retry, the filter clear and the details toggle: what the state view's buttons call.

use gpui::Context;
use oxikube_app::store::StoreFilter;
use oxikube_domain::command::Command;

use crate::table::view::{ResourceTable, ResourceTableEvent};

impl ResourceTable {
    /// The Retry button: dispatches `resource::RetryFeed`, which the bus hands back to
    /// [`retry_feed`](Self::retry_feed) (so the palette, a key and an agent run the same thing).
    pub fn request_retry(&mut self, cx: &mut Context<Self>) {
        let command = Command::ResourceRetryFeed {
            cluster: self.cluster.clone(),
            gvk: self.kind.gvk.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Restarts the feed behind this table (what `resource::RetryFeed` does): follows a
    /// reconnect to the session's new store first, then asks the store to reopen every feed that
    /// is not ready. The rows stay, marked stale, until the new list reconciles them.
    pub fn retry_feed(&mut self, cx: &mut Context<Self>) {
        self.resubscribe(cx);
        if let Some(subscription) = &mut self.subscription {
            subscription.retry();
        }
        // The store wakes the poll task; this redraws the button press at once.
        cx.notify();
    }

    /// Applies an in-app filter to the rows (the filter bar's, E07-S04). `label` is how the
    /// filter reads in the "no matches" state.
    pub fn set_filter(
        &mut self,
        filter: StoreFilter,
        label: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let label = label.filter(|_| !filter.is_empty());
        self.filter = filter.clone();
        if let Some(subscription) = &mut self.subscription {
            subscription.set_filter(filter);
        }
        self.table.update_quiet(cx, |d| d.filter = label);
        cx.notify();
    }

    /// Clears the filter (the state view's "Clear filter"), and tells the owner so the filter bar
    /// follows.
    pub fn clear_filter(&mut self, cx: &mut Context<Self>) {
        self.set_filter(StoreFilter::default(), None, cx);
        cx.emit(ResourceTableEvent::FilterCleared);
    }

    /// Expands or collapses the failure detail of the state view.
    pub fn toggle_state_details(&mut self, cx: &mut Context<Self>) {
        self.table
            .update_quiet(cx, |d| d.details_open = !d.details_open);
        cx.notify();
    }
}
