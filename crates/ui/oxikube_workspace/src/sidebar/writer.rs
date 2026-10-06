//! [`SidebarWriter`]: the one task that writes a cluster's sidebar state, in order.
//!
//! Every toggle sends its full snapshot of the user's choices down a channel; one background task
//! drains it, so writes reach the `StatePort` in the order the user made them and a burst of
//! toggles (a held key) collapses into one write of the newest snapshot. The task first reads
//! what was saved and writes each snapshot on top of it, so a toggle made before the panel's own
//! load returned does not drop the choices that were saved earlier.
//!
//! The task is detached on purpose: it ends when the panel (the sender) is dropped, after it has
//! written what was still queued, so a click right before the tab closes is not lost. It holds no
//! entity, so there is nothing for it to drop itself from.

use std::collections::BTreeMap;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, AppContext as _};

use super::store::SidebarStore;

/// Sends open and closed snapshots to the task that persists them. See the [module docs](self).
pub(super) struct SidebarWriter {
    tx: UnboundedSender<BTreeMap<String, bool>>,
}

impl SidebarWriter {
    /// Starts the writer task for `store`.
    pub(super) fn spawn(store: SidebarStore, cx: &App) -> Self {
        let (tx, mut rx) = unbounded::<BTreeMap<String, bool>>();
        cx.background_spawn(async move {
            let mut saved = match store.load().await {
                Ok(saved) => saved.map(|s| s.open).unwrap_or_default(),
                Err(error) => {
                    tracing::warn!(%error, "reading the sidebar state before saving failed");
                    BTreeMap::new()
                }
            };
            while let Some(mut snapshot) = rx.next().await {
                // Only the newest snapshot matters: it holds every choice made so far.
                while let Ok(newer) = rx.try_recv() {
                    snapshot = newer;
                }
                let mut row = saved.clone();
                row.extend(snapshot);
                match store.save(&row).await {
                    Ok(()) => saved = row,
                    Err(error) => tracing::warn!(%error, "saving the sidebar state failed"),
                }
            }
        })
        .detach();
        Self { tx }
    }

    /// Queues `open` (every choice the user made this session) to be written.
    pub(super) fn save(&self, open: BTreeMap<String, bool>) {
        // The task outlives every sender, so this only fails once it is gone.
        self.tx.unbounded_send(open).ok();
    }
}
