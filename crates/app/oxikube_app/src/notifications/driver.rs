//! The event side of the centre: one [`Watch`] per open, unmuted cluster, whose pump reads the
//! cluster's Warning events from the `EventService` (never a watch of its own) and hands the news
//! through the rules into the centre; and the follower that keeps the watches in step with the
//! sessions.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use futures::StreamExt as _;
use oxikube_domain::event::Event;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::notification::NotificationKind;

use super::service::{Inner, wake};
use super::tracker::Tracker;
use crate::events::{EventsChanged, EventsSubscription};
use crate::session::SessionChange;
use crate::store::{TaskGuard, spawn_guarded};

/// One cluster watched for notifications: which of its events were told, and the pump task
/// (abort-on-drop; dropping it drops the events subscription).
pub(crate) struct Watch {
    pub(crate) tracker: Tracker,
    pub(crate) pump: Option<TaskGuard>,
    /// The pump the tracker belongs to: what an older pump still delivers is dropped.
    pub(crate) pump_id: u64,
}

/// Numbers the pumps, so a pump replaced while it was handing over a change is told apart.
static NEXT_PUMP: AtomicU64 = AtomicU64::new(1);

impl Watch {
    /// Drops the pump so [`reconcile`] subscribes again (the event filter changed), with a
    /// tracker started at `now`: the new subscription's first reset holds events the old filter
    /// never showed (a newly watched kind's warnings already in the buffer), which are backlog,
    /// not news; a kept tracker would judge them as a relist and tell them all.
    pub(crate) fn resubscribe(&mut self, now: jiff::Timestamp) {
        self.pump = None;
        self.pump_id = 0;
        self.tracker = Tracker::new(now);
    }
}

/// Follows the session updates until the service is dropped.
pub(crate) fn follow(inner: &Arc<Inner>) -> impl Future<Output = ()> + Send + 'static {
    let updates = inner.sessions.subscribe();
    let weak = Arc::downgrade(inner);
    async move {
        let mut updates = updates;
        // The sessions opened before the follower subscribed.
        if let Some(inner) = weak.upgrade() {
            reconcile_all(&inner);
        }
        while let Some(item) = updates.next().await {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            match item {
                Ok(update) => match update.change {
                    SessionChange::Opened | SessionChange::Closed => {
                        reconcile(&inner, &update.cluster);
                    }
                    _ => {}
                },
                Err(_lagged) => reconcile_all(&inner),
            }
        }
    }
}

/// Brings every cluster's watch in step: the open sessions, and the watches of closed ones.
pub(crate) fn reconcile_all(inner: &Arc<Inner>) {
    let mut clusters: Vec<ClusterId> = inner
        .sessions
        .sessions()
        .iter()
        .map(|session| session.id().clone())
        .collect();
    clusters.extend(inner.state.lock().watches.keys().cloned());
    clusters.sort();
    clusters.dedup();
    for cluster in &clusters {
        reconcile(inner, cluster);
    }
}

/// Watches `cluster` when it is open, not muted, the muted list was read and a spawner is
/// attached; forgets it when it is closed or muted.
pub(crate) fn reconcile(inner: &Arc<Inner>, cluster: &ClusterId) {
    let open = inner.sessions.get(cluster).is_some();
    let mut state = inner.state.lock();
    if !open || state.muted.contains(cluster) {
        if state.watches.remove(cluster).is_some() {
            state.limiter.forget(cluster);
        }
        return;
    }
    let Some(spawner) = state.spawner.clone().filter(|_| state.loaded) else {
        return;
    };
    let now = inner.clock.now();
    let filter = state.rules.event_filter(cluster);
    let watch = state
        .watches
        .entry(cluster.clone())
        .or_insert_with(|| Watch {
            tracker: Tracker::new(now),
            pump: None,
            pump_id: 0,
        });
    if watch.pump.is_some() {
        return;
    }
    let pump_id = NEXT_PUMP.fetch_add(1, Ordering::Relaxed);
    watch.pump_id = pump_id;
    let subscription = inner.events.subscribe(cluster, filter);
    let task = pump(
        Arc::downgrade(inner),
        cluster.clone(),
        pump_id,
        subscription,
    );
    watch.pump = Some(spawn_guarded(&spawner, task));
}

/// Reads one cluster's events into the centre until aborted or the service is dropped.
async fn pump(
    inner: Weak<Inner>,
    cluster: ClusterId,
    pump_id: u64,
    mut subscription: EventsSubscription,
) {
    while let Some(changes) = subscription.next().await {
        let Some(inner) = inner.upgrade() else {
            return;
        };
        let held = if changes.reset {
            subscription.query()
        } else {
            Vec::new()
        };
        on_events(&inner, &cluster, pump_id, changes, &held);
    }
}

/// Hands the news among `changes` (and the `held` events of a reset) to the centre, unless pump
/// `pump_id` was replaced meanwhile (its changes would count against the new tracker).
pub(crate) fn on_events(
    inner: &Inner,
    cluster: &ClusterId,
    pump_id: u64,
    changes: EventsChanged,
    held: &[Arc<Event>],
) {
    let now = inner.clock.now();
    let mut wakers = Vec::new();
    {
        let mut state = inner.state.lock();
        let state = &mut *state;
        if state.muted.contains(cluster) {
            return;
        }
        let Some(watch) = state
            .watches
            .get_mut(cluster)
            .filter(|watch| watch.pump_id == pump_id)
        else {
            return;
        };
        let mut news = watch.tracker.reset(held);
        for key in &changes.removed {
            watch.tracker.forget(key);
        }
        for event in changes.added.iter().chain(&changes.updated) {
            if let Some(occurrences) = watch.tracker.observe(event) {
                news.push((event.clone(), occurrences));
            }
        }
        if !state.accepts(NotificationKind::ClusterEvent, Some(cluster)) {
            return;
        }
        for (event, occurrences) in news {
            if !state.rules.admits(cluster, &event) {
                continue;
            }
            let input = state.rules.input_for(cluster, &event, occurrences);
            let applied = state
                .centre
                .apply(input, &mut state.limiter, &state.rules, now);
            wakers.extend(state.publish(applied.raised.then_some(&applied.row)));
        }
    }
    wake(wakers);
}
