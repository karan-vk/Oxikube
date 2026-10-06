//! Retry, the filter clear and the details toggle: what the state view's buttons call.

use gpui::{Context, Window};
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

    /// Clears the filter (the state view's "Clear filter"): the bar empties, which applies the
    /// empty filter, and the owner is told.
    pub fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |bar, cx| bar.clear(window, cx));
        cx.emit(ResourceTableEvent::FilterCleared);
    }

    /// Expands or collapses the failure detail of the state view.
    pub fn toggle_state_details(&mut self, cx: &mut Context<Self>) {
        self.table
            .update_quiet(cx, |d| d.details_open = !d.details_open);
        cx.notify();
    }
}
