//! Following the session: the store, the lease on the tiles' kinds, the metrics interest, the
//! once-a-second tick.
//!
//! A page nobody looks at costs nothing: while it is not the displayed tab of its pane (or its
//! cluster is not connected) it holds no lease, no metrics interest and counts nothing. The store
//! stops the feeds after its grace period unless a table holds them, and the metrics service
//! stops polling when its last interest goes.

use futures::StreamExt as _;
use gpui::{Context, WeakEntity};
use oxikube_app::{CountState, MetricsKind, SessionChange};
use oxikube_runtime::notify_coalesced;

use super::fallback::Recount;
use super::view::PulsesView;
use super::{HISTORY_INTERVAL, REFRESH_INTERVAL, SETTLE_TICKS};

impl PulsesView {
    /// Starts what the page follows: the session, the metrics polls and the tick.
    pub(super) fn start(&mut self, cx: &mut Context<Self>) {
        // Subscribe before reading the session, so no update falls between the two.
        let mut updates = self.deps.sessions.subscribe();
        let cluster = self.cluster.clone();
        self.tasks
            .push(cx.spawn(async move |this: WeakEntity<Self>, cx| {
                while let Some(item) = updates.next().await {
                    let alive = this.update(cx, |this, cx| match item {
                        Ok(update) if update.cluster == cluster => {
                            this.on_change(&update.change, cx)
                        }
                        Ok(_) => {}
                        // Missed some: look at the session as it is now.
                        Err(_) => this.sync(cx),
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            }));
        self.tasks
            .push(cx.spawn(async move |this: WeakEntity<Self>, cx| {
                loop {
                    cx.background_executor().timer(REFRESH_INTERVAL).await;
                    if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                        break;
                    }
                }
            }));
        if let Some(metrics) = self.deps.metrics.clone() {
            // One wake-up per poll, not per sample.
            let mut changes = metrics.changes(&self.cluster);
            self.tasks
                .push(cx.spawn(async move |this: WeakEntity<Self>, cx| {
                    while changes.next().await.is_some() {
                        if this
                            .update(cx, |this, cx| this.refresh_cluster(cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                }));
        }
        self.sync(cx);
    }

    fn on_change(&mut self, change: &SessionChange, cx: &mut Context<Self>) {
        match change {
            // A first connect, a reconnect or a disconnect: the store may be a new one, or gone;
            // a new selection changes what is counted.
            SessionChange::StateChanged { .. }
            | SessionChange::NamespaceChanged(_)
            | SessionChange::Closed => self.sync(cx),
            _ => {}
        }
    }

    /// Finds the session's store and keeps a lease on the tiles' kinds under the current
    /// namespace selection, and the metrics interest, while the page is displayed. Cheap when
    /// nothing changed.
    pub(super) fn sync(&mut self, cx: &mut Context<Self>) {
        let session = self
            .deps
            .sessions
            .get(&self.cluster)
            .filter(|s| s.is_connected());
        let store = session
            .as_ref()
            .and_then(|s| self.deps.stores.for_session(s));
        let (Some(session), Some(store)) = (session, store) else {
            self.release();
            self.model
                .set_counts(&vec![CountState::NotWatched; self.model.targets().len()]);
            cx.notify();
            return;
        };
        self.store = Some(store.clone());
        if !self.active {
            // Hidden: hold nothing. What the page last showed stays until it is shown again.
            self.release_feeds();
            return;
        }
        let selection = session.namespace_selection().clone();
        let targets = self.model.targets();
        match &mut self.lease {
            Some(lease) if lease.targets() == targets.as_slice() => {
                lease.rescope(&selection);
            }
            _ => {
                self.lease = Some(store.lease_counts(targets, &selection));
                self.age = 0;
            }
        }
        if self.interest.is_none()
            && let Some(metrics) = &self.deps.metrics
        {
            self.interest = Some(metrics.watch(&self.cluster, MetricsKind::Nodes));
        }
        self.refresh_counts(cx);
        self.refresh_cluster(cx);
        cx.notify();
    }

    /// Lets go of everything the page holds on the cluster.
    pub(super) fn release(&mut self) {
        self.store = None;
        self.release_feeds();
    }

    fn release_feeds(&mut self) {
        self.lease = None;
        self.interest = None;
        self.gauge_task = None;
        self.gauge_stamp = None;
        self.recounts.iter_mut().for_each(Recount::cancel);
    }

    /// The once-a-second work: read the counts, take a history sample when one is due, ask for
    /// the one-off counts that are due.
    pub(super) fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.active || self.lease.is_none() {
            return;
        }
        self.age = self.age.saturating_add(1);
        self.refresh_counts(cx);
        let now = self.now();
        if self
            .last_sample
            .is_none_or(|last| now - last >= HISTORY_INTERVAL.as_secs_f64())
            && self.model.sample(now)
        {
            self.last_sample = Some(now);
            notify_coalesced(cx);
        }
        self.fetch_due(now, cx);
        // Nodes that were not listed when the metrics arrived.
        self.refresh_cluster(cx);
    }

    /// Reads every tile's count from the lease; redraws (coalesced) when one changed.
    pub(super) fn refresh_counts(&mut self, cx: &mut Context<Self>) {
        let Some(lease) = &self.lease else {
            return;
        };
        let states = lease.counts();
        if self.model.set_counts(&states) {
            notify_coalesced(cx);
        }
    }

    /// Whether the feeds had time to start, so a kind still not watched is really not.
    pub(super) fn settled(&self) -> bool {
        self.age >= SETTLE_TICKS
    }

    /// The clock's time in seconds since the epoch.
    pub(super) fn now(&self) -> f64 {
        self.deps.clock.now().as_second() as f64
    }
}
