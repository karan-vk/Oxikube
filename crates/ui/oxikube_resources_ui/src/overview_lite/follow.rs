//! Following the session: the store, the lease on the tile kinds, the timer.

use futures::StreamExt as _;
use gpui::{Context, WeakEntity};
use oxikube_app::SessionChange;
use oxikube_runtime::notify_coalesced;

use super::REFRESH_INTERVAL;
use super::tiles::TileRegistry;
use super::view::WorkloadsOverview;
use super::warnings::WARNING_ROWS;

impl WorkloadsOverview {
    /// Starts what the overview follows: the tile registry, the session, the refresh timer.
    pub(super) fn start(&mut self, cx: &mut Context<Self>) {
        self.reload_tiles(cx);
        // A tile registered after the overview opened appears.
        self.subscriptions
            .push(cx.observe_global::<TileRegistry>(|this, cx| {
                this.reload_tiles(cx);
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

    /// Finds the session's store and keeps the counters on it under the current namespace
    /// selection; reads the numbers now. Cheap when nothing changed.
    pub(super) fn sync(&mut self, cx: &mut Context<Self>) {
        let session = self
            .deps
            .sessions
            .get(&self.cluster)
            .filter(|s| s.is_connected());
        let store = session
            .as_ref()
            .and_then(|s| self.deps.stores.for_session(s));
        match (&session, &store) {
            (Some(session), Some(store)) => {
                let selection = session.namespace_selection().clone();
                self.counters.follow(Some((store, &selection)));
            }
            _ => {
                self.counters.follow(None);
            }
        }
        self.refresh_warnings();
        cx.notify();
    }

    /// Reads every tile's count and the warnings again; redraws (coalesced) when one changed.
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        let counts = self.counters.refresh();
        let warnings = self.refresh_warnings();
        if counts || warnings {
            notify_coalesced(cx);
        }
    }

    /// Asks the warnings hook for the newest rows; whether they differ from the ones shown. Not
    /// connected means no warnings.
    fn refresh_warnings(&mut self) -> bool {
        let rows = if self.counters.is_open() {
            self.deps.warnings.latest(&self.cluster, WARNING_ROWS)
        } else {
            Vec::new()
        };
        if rows == self.warnings {
            return false;
        }
        self.warnings = rows;
        true
    }
}
