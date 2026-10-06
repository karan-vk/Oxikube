//! Following the session manager: tabs open, refresh and close with their sessions.

use gpui::{AppContext as _, Context, Window};
use oxikube_app::{ClusterSession, SessionChange, SessionUpdate};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::SessionPhase;
use oxikube_runtime::NotifyCoalescedExt as _;

use super::{ClusterTabs, ClusterTabsEvent, TabEntry};
use crate::cluster::ClusterMark;
use crate::cluster_tab::{
    store::cluster_layout_key,
    tab::{ClusterTab, ClusterTabEvent, ClusterTabInfo},
};
use crate::persistence::{LayoutPersistence, LayoutStore};
use crate::workspace::Workspace;

impl ClusterTabs {
    /// Applies one session update. Cheap for the many updates that change nothing a tab shows.
    pub(super) fn apply_session_update(
        &mut self,
        update: SessionUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match update.change {
            SessionChange::Closed => self.close_tab_now(&update.cluster, window, cx),
            SessionChange::StateChanged { .. }
            | SessionChange::ColourChanged(_)
            | SessionChange::ReadOnlyChanged(_)
            | SessionChange::DisplayNameChanged(_) => {
                match self.deps.sessions.get(&update.cluster) {
                    Some(session) => self.follow(&session, window, cx),
                    None => self.close_tab_now(&update.cluster, window, cx),
                }
            }
            // Opened (still disconnected), capabilities and namespaces change nothing a tab shows.
            _ => {}
        }
    }

    /// Reads every session again (the update stream lagged, or the controller just started).
    pub(super) fn sync_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sessions = self.deps.sessions.sessions();
        let stale: Vec<ClusterId> = self
            .tabs
            .keys()
            .filter(|cluster| sessions.iter().all(|s| s.id() != *cluster))
            .cloned()
            .collect();
        for cluster in stale {
            self.close_tab_now(&cluster, window, cx);
        }
        for session in sessions {
            self.follow(&session, window, cx);
        }
    }

    /// Opens the tab of a session that is not disconnected, refreshes it when it exists, closes
    /// it when the session disconnected. A restored placeholder
    /// ([`is_placeholder`](ClusterTabs::is_placeholder)) is the exception: its session is
    /// disconnected and its tab stays, until the cluster connects.
    pub(super) fn follow(
        &mut self,
        session: &ClusterSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cluster = session.id().clone();
        if session.phase() != SessionPhase::Disconnected {
            // Connecting or connected: no longer a placeholder, an ordinary tab.
            self.pending.remove(&cluster);
        } else if !self.pending.contains(&cluster) {
            self.close_tab_now(&cluster, window, cx);
            return;
        }
        if let Some(entry) = self.tabs.get(&cluster) {
            let info = info_of(session);
            entry.tab.update(cx, |tab, cx| tab.set_info(info, cx));
            // The hotbar and the title read the same session: redraw within a frame.
            cx.notify_coalesced();
        } else {
            self.open_tab(session, window, cx);
        }
    }

    fn open_tab(&mut self, session: &ClusterSession, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let cluster = session.id().clone();
        let layers = workspace.read(cx).layers();
        let inner = cx.new(|cx| Workspace::embedded(layers, window, cx));
        let info = info_of(session);
        let tab = cx.new(|cx| ClusterTab::new(cluster.clone(), info, inner.clone(), cx));

        // The setup adds the sidebar panel and registers item builders, which the layout
        // restore below needs, so it runs first.
        if let Some(setup) = self.deps.setup.clone() {
            setup(&tab, session, window, cx);
        }
        match LayoutStore::new(self.deps.state.clone(), &cluster_layout_key(&cluster)) {
            Ok(store) => {
                let persistence = LayoutPersistence::start_embedded(&inner, store, window, cx);
                tab.update(cx, |tab, _| tab.set_persistence(persistence));
            }
            Err(error) => tracing::warn!(%error, %cluster, "the cluster layout is not saved"),
        }

        let subscription = {
            let cluster = cluster.clone();
            cx.subscribe_in(
                &tab,
                window,
                move |this, _, event: &ClusterTabEvent, window, cx| {
                    this.on_tab_event(&cluster, *event, window, cx)
                },
            )
        };
        let item = workspace.update(cx, |ws, cx| ws.open_item(tab.clone(), window, cx));
        self.tabs.insert(
            cluster.clone(),
            TabEntry {
                tab,
                item,
                _subscription: subscription,
            },
        );
        cx.emit(ClusterTabsEvent::Opened(cluster));
        cx.emit(ClusterTabsEvent::OrderChanged);
        self.mark_dirty(cx);
        cx.notify();
    }

    /// Closes `cluster`'s tab at once, without asking: its session is gone or disconnected.
    pub(super) fn close_tab_now(
        &mut self,
        cluster: &ClusterId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.tabs.get(cluster) else {
            return;
        };
        let item = entry.item;
        if let Some(workspace) = self.workspace.upgrade() {
            // `on_close` of the tab reports back through `ClusterTabEvent::Closed`, which forgets
            // the entry.
            workspace.update(cx, |ws, cx| ws.close_item(item, window, cx));
        }
    }

    pub(super) fn on_tab_event(
        &mut self,
        cluster: &ClusterId,
        event: ClusterTabEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ClusterTabEvent::ActiveChanged(true) => {
                self.set_active(Some(cluster.clone()), cx);
                self.on_displayed(cluster, cx);
            }
            ClusterTabEvent::ActiveChanged(false) => {
                if self.active.as_ref() == Some(cluster) {
                    self.set_active(None, cx);
                }
            }
            ClusterTabEvent::CloseRequested => self.request_close(cluster, window, cx),
            ClusterTabEvent::Closed => self.forget(cluster, cx),
        }
    }

    fn forget(&mut self, cluster: &ClusterId, cx: &mut Context<Self>) {
        self.pending.remove(cluster);
        if self.tabs.shift_remove(cluster).is_none() {
            return;
        }
        if self.active.as_ref() == Some(cluster) {
            self.active = None;
            cx.emit(ClusterTabsEvent::ActiveChanged(None));
        }
        cx.emit(ClusterTabsEvent::Closed(cluster.clone()));
        cx.emit(ClusterTabsEvent::OrderChanged);
        self.mark_dirty(cx);
        cx.notify();
    }
}

/// What a tab shows about `session`.
fn info_of(session: &ClusterSession) -> ClusterTabInfo {
    ClusterTabInfo {
        title: session.title().to_owned().into(),
        mark: ClusterMark::of(session),
        state: session.state().clone(),
    }
}
