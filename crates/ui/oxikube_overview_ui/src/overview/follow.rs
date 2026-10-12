//! Following the session, and starting or stopping the live part: the feed's driver (off the UI
//! thread), the publish listener (coalesced redraws), the version read, the warnings timer and the
//! history graphs.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{AppContext as _, Context, WeakEntity};
use oxikube_app::cluster_overview::{OverviewFeed, OverviewInput, TOP_N};
use oxikube_app::{ClusterSession, SessionChange};
use oxikube_domain::OxiError;
use oxikube_runtime::{notify_coalesced, spawn_kube};

use super::info::VersionState;
use super::view::{ClusterOverview, Live, Shown, WARNINGS_SHOWN};

/// The shortest time between two redraws for publishes: a churning cluster publishes often, the
/// tiles need not move more than ten times a second.
const REDRAW_EVERY: Duration = Duration::from_millis(100);
/// How often the warnings hook is read while the overview is on screen.
const WARNINGS_EVERY: Duration = Duration::from_secs(1);

impl ClusterOverview {
    /// Starts following the session (for the item's whole life) and syncs once.
    pub(super) fn start(&mut self, cx: &mut Context<Self>) {
        // Subscribe before reading the session, so no update falls between the two.
        let mut updates = self.deps.sessions.subscribe();
        let cluster = self.cluster.clone();
        self.tasks
            .push(cx.spawn(async move |this: WeakEntity<Self>, cx| {
                while let Some(item) = updates.next().await {
                    let alive = this.update(cx, |this, cx| match item {
                        Ok(update) if update.cluster == cluster => match update.change {
                            SessionChange::StateChanged { .. } | SessionChange::Closed => {
                                this.sync(cx)
                            }
                            _ => {}
                        },
                        Ok(_) => {}
                        // Missed some: look at the session as it is now.
                        Err(_) => this.sync(cx),
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            }));
        self.sync(cx);
    }

    /// Follows the session as it is now: the info card's fields, and the live part running
    /// exactly while the cluster is connected, the item displayed and its cluster tab shown.
    pub(super) fn sync(&mut self, cx: &mut Context<Self>) {
        let session = self.deps.sessions.get(&self.cluster);
        if let Some(session) = &session {
            self.context = session.context().to_string();
            self.server = session.server().map(str::to_owned);
        }
        let session = session.filter(ClusterSession::is_connected);
        if self.connected != session.is_some() {
            // A new connection may be another server: read its version again.
            self.version = VersionState::Loading;
        }
        self.connected = session.is_some();
        let store = session
            .as_ref()
            .filter(|_| self.active && self.tab_shown)
            .and_then(|s| self.deps.stores.for_session(s));
        match (store, &self.live) {
            (Some(store), Some(live)) if live.store.is_same_store(&store) => {}
            (Some(store), _) => {
                let session = session.expect("a store comes from a connected session");
                self.start_live(store, &session, cx);
            }
            (None, _) => {
                self.live = None;
                self.history.stop();
            }
        }
        cx.notify();
    }

    fn start_live(
        &mut self,
        store: oxikube_app::store::ResourceStore,
        session: &ClusterSession,
        cx: &mut Context<Self>,
    ) {
        // The old feed's tasks go first, so its feeds are released before the new ones ask.
        self.live = None;
        let started = OverviewFeed::start(OverviewInput {
            cluster: self.cluster.clone(),
            store: store.clone(),
            metrics: self.deps.metrics.clone(),
            top_n: TOP_N,
        });
        let feed = started.feed.clone();
        if let Some(version) = self.version.known() {
            feed.set_server_version(Some(version.to_owned()));
        }
        let mut tasks = vec![cx.background_spawn(started.driver)];
        let mut changes = started.changes;
        tasks.push(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while changes.next().await.is_some() {
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(REDRAW_EVERY).await;
            }
        }));
        tasks.push(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            loop {
                if this.update(cx, |this, cx| this.read_warnings(cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(WARNINGS_EVERY).await;
            }
        }));
        if matches!(self.version, VersionState::Loading) {
            tasks.extend(self.read_version(session, cx));
        }
        self.live = Some(Live {
            store,
            feed,
            _tasks: tasks,
        });
        self.history.start(self.deps.history.clone(), cx);
    }

    /// Reads the latest snapshot; rebuilds what is drawn when it moved.
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(live) = &self.live else {
            return;
        };
        let snapshot = live.feed.snapshot();
        if !Arc::ptr_eq(&snapshot, &self.shown.snapshot) {
            let shown = Shown::of(snapshot, self.deps.metrics.is_some());
            if shown != self.shown {
                self.shown = shown;
                self.history.set_capacity(&self.shown.snapshot, cx);
                notify_coalesced(cx);
            }
        }
    }

    fn read_warnings(&mut self, cx: &mut Context<Self>) {
        let warnings = self.deps.warnings.latest(&self.cluster, WARNINGS_SHOWN);
        if warnings != self.warnings {
            self.warnings = warnings;
            notify_coalesced(cx);
        }
    }

    /// Reads the API server's version once (on the Tokio bridge).
    fn read_version(
        &mut self,
        session: &ClusterSession,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Task<()>> {
        let discovery = session.discovery()?;
        let read = spawn_kube(cx, async move { discovery.server_version().await });
        Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let result = match read.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::from(error)),
            };
            this.update(cx, |this, cx| {
                this.version = match result {
                    Ok(version) if !version.git_version.is_empty() => {
                        VersionState::Known(version.git_version)
                    }
                    Ok(version) => {
                        VersionState::Known(format!("v{}.{}", version.major, version.minor))
                    }
                    Err(error) => VersionState::Failed(error.message().to_owned()),
                };
                if let Some(live) = &this.live {
                    live.feed
                        .set_server_version(this.version.known().map(str::to_owned));
                }
                cx.notify();
            })
            .ok();
        }))
    }
}
