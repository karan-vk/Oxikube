//! Following the session: the store, the lease on the tile kinds, the timer.

use futures::StreamExt as _;
use gpui::{Context, WeakEntity};
use oxikube_app::{CountState, CountTarget, SessionChange};
use oxikube_runtime::notify_coalesced;

use super::REFRESH_INTERVAL;
use super::tiles::TileRegistry;
use super::view::WorkloadsOverview;

impl WorkloadsOverview {
    /// Starts what the overview follows: the tile registry, the session, the refresh timer.
    pub(super) fn start(&mut self, cx: &mut Context<Self>) {
        self.reload_tiles(cx);
        // A tile registered after the overview opened appears.
        self.subscriptions
            .push(cx.observe_global::<TileRegistry>(|this, cx| {
                this.reload_tiles(cx);
                this.lease = None;
                this.sync(cx);
            }));

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
                    if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                        break;
                    }
                }
            }));
        self.sync(cx);
    }

    fn on_change(&mut self, change: &SessionChange, cx: &mut Context<Self>) {
        match change {
            // A first connect, a reconnect or a disconnect: the store may be a new one, or gone.
            SessionChange::StateChanged { .. }
            | SessionChange::NamespaceChanged(_)
            | SessionChange::Closed => self.sync(cx),
            _ => {}
        }
    }

    /// Finds the session's store and keeps a lease on the tiles' kinds under the current
    /// namespace selection; reads the numbers now. Cheap when nothing changed.
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
            self.store = None;
            self.lease = None;
            self.set_states(vec![CountState::NotWatched; self.tiles.len()], cx);
            cx.notify();
            return;
        };
        let selection = session.namespace_selection().clone();
        let targets: Vec<CountTarget> = self.tiles.iter().map(|t| t.target.clone()).collect();
        let same_store = self.store.as_ref().is_some_and(|s| s.is_same_store(&store));
        match &mut self.lease {
            Some(lease) if same_store && lease.targets() == targets.as_slice() => {
                lease.rescope(&selection);
            }
            _ => self.lease = Some(store.lease_counts(targets, &selection)),
        }
        self.store = Some(store);
        self.refresh(cx);
        cx.notify();
    }

    /// Reads every tile's count from the lease; redraws (coalesced) when one changed.
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(lease) = &self.lease else {
            return;
        };
        let states = lease.counts();
        self.set_states(states, cx);
    }

    fn set_states(&mut self, states: Vec<CountState>, cx: &mut Context<Self>) {
        if states != self.states {
            self.states = states;
            notify_coalesced(cx);
        }
    }
}
