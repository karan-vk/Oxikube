//! [`Hotbar`]: the GPUI entity of the strip, its loading, and what its tiles do.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use futures::{Stream, StreamExt as _};
use gpui::{
    Context, Entity, FocusHandle, Focusable, Subscription, Task, UniformListScrollHandle,
    WeakEntity, Window,
};
use oxikube_app::{
    ClusterCatalog, ClusterSession, ClusterSessionManager, FavouriteChanged, FavouritesLagged,
    SessionChange, SessionUpdate,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::StatePort;
use oxikube_runtime::{KubeTaskError, NotifyCoalescedExt as _, spawn_kube};
use oxikube_workspace::persistence::MAIN_WINDOW_ID;
use oxikube_workspace::{ClusterTabs, ClusterTabsEvent, CommandDispatcher};

use super::model::{HotbarModel, SessionLook};
use super::store::HotbarStore;

/// What the hotbar needs from the outside. All of it is handed in, so a test builds the strip
/// over fakes and the app builds it over its adapters.
#[derive(Clone)]
pub struct HotbarDeps {
    /// The favourites (and their names).
    pub catalog: ClusterCatalog,
    /// The sessions whose colours and states the tiles show. Only read here.
    pub sessions: ClusterSessionManager,
    /// The window's cluster tabs: which one is displayed.
    pub tabs: Entity<ClusterTabs>,
    /// Where the commands of a click or of the context menu go. Give it a
    /// [`TabsDispatcher`](oxikube_workspace::cluster_tab::TabsDispatcher) over the catalog's
    /// dispatcher, so `cluster::Select` reaches the tabs and `cluster::Connect` the sessions.
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// Where the user's order is kept.
    pub state: Arc<dyn StatePort>,
    /// The window's id in the state store ([`MAIN_WINDOW_ID`] for the main window).
    pub window_id: String,
}

impl HotbarDeps {
    /// Dependencies for the main window.
    pub fn new(
        catalog: ClusterCatalog,
        sessions: ClusterSessionManager,
        tabs: Entity<ClusterTabs>,
        dispatcher: Rc<dyn CommandDispatcher>,
        state: Arc<dyn StatePort>,
    ) -> Self {
        Self {
            catalog,
            sessions,
            tabs,
            dispatcher,
            state,
            window_id: MAIN_WINDOW_ID.to_owned(),
        }
    }
}

/// The hotbar. See the [module docs](super).
pub struct Hotbar {
    pub(super) deps: HotbarDeps,
    pub(super) model: HotbarModel,
    store: Option<HotbarStore>,
    pub(super) scroll: UniformListScrollHandle,
    pub(super) focus: FocusHandle,
    /// The catalog read in flight. A newer read replaces (and so cancels) it from outside.
    load: Option<Task<()>>,
    /// The order write in flight; the next drop replaces it.
    save: Option<Task<()>>,
    /// Reads the saved order once. Lives as long as the view.
    _load_order: Task<()>,
    /// Applies session updates. Lives as long as the view.
    _watch_sessions: Task<()>,
    /// Applies favourite changes. Lives as long as the view.
    _watch_favourites: Task<()>,
    /// Re-reads the catalog when the sources change. Lives as long as the view.
    _watch_sources: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Hotbar {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Hotbar {
    /// Builds the strip and starts reading the catalog and the saved order in the background.
    /// The first frame shows the connected clusters (the session manager answers at once) and
    /// the favourites follow when the read ends.
    pub fn new(deps: HotbarDeps, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = match HotbarStore::new(deps.state.clone(), &deps.window_id) {
            Ok(store) => Some(store),
            Err(error) => {
                tracing::warn!(%error, "the hotbar order is not saved");
                None
            }
        };

        // Subscribe before taking the snapshot, so no update falls between the two.
        let watch_sessions = watch(deps.sessions.subscribe(), cx, |this, item, cx| match item {
            Ok(update) => this.apply_session_update(update, cx),
            // Missed some: re-read every session instead of replaying.
            Err(_) => this.sync_sessions(cx),
        });
        let watch_favourites = watch(
            deps.catalog.favourite_changes(),
            cx,
            Self::on_favourite_change,
        );
        let watch_sources = watch(deps.catalog.changes(), cx, |this, _, cx| this.reload(cx));
        let load_order = {
            let store = store.clone();
            cx.spawn(async move |this: WeakEntity<Self>, cx| {
                let Some(store) = store else { return };
                let Some(order) = log_failure(
                    spawn_kube(&*cx, async move { store.load().await }).await,
                    "reading the saved hotbar order",
                ) else {
                    return;
                };
                this.update(cx, |this, cx| {
                    // An order the user placed while this was reading wins (the model ignores this one).
                    if this.model.restore_order(order) {
                        cx.notify_coalesced();
                    }
                })
                .ok();
            })
        };
        let subscriptions =
            vec![
                cx.subscribe(&deps.tabs, |this, _, event: &ClusterTabsEvent, cx| {
                    if let ClusterTabsEvent::ActiveChanged(active) = event
                        && this.model.set_active(active.clone())
                    {
                        cx.notify();
                    }
                }),
            ];

        let mut view = Self {
            deps,
            model: HotbarModel::new(),
            store,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            load: None,
            save: None,
            _load_order: load_order,
            _watch_sessions: watch_sessions,
            _watch_favourites: watch_favourites,
            _watch_sources: watch_sources,
            _subscriptions: subscriptions,
        };
        let active = view.deps.tabs.read(cx).active().cloned();
        view.model.set_active(active);
        view.sync_sessions(cx);
        view.reload(cx);
        view
    }

    /// The tiles' data, for tests and for the status of the strip.
    pub fn model(&self) -> &HotbarModel {
        &self.model
    }

    /// Reads the catalog again for the favourites (the sources changed, or the caller asks). The
    /// previous read, if still running, is dropped.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let catalog = self.deps.catalog.clone();
        self.load = Some(cx.spawn(async move |this, cx| {
            let Some(entries) = log_failure(
                spawn_kube(&*cx, async move { catalog.load().await }).await,
                "reading the catalog for the hotbar",
            ) else {
                return;
            };
            this.update(cx, |this, cx| {
                let favourites: HashMap<ClusterId, String> = entries
                    .iter()
                    .filter(|entry| entry.favourite)
                    .map(|entry| (entry.id().clone(), entry.name().to_owned()))
                    .collect();
                if this.model.set_favourites(favourites) {
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn sync_sessions(&mut self, cx: &mut Context<Self>) {
        let sessions = self.deps.sessions.sessions();
        let mut changed = self
            .model
            .retain_sessions(|cluster| sessions.iter().any(|s| s.id() == cluster));
        for session in &sessions {
            changed |= self
                .model
                .set_session(session.id().clone(), look_of(session));
        }
        if changed {
            cx.notify_coalesced();
        }
    }

    fn apply_session_update(&mut self, update: SessionUpdate, cx: &mut Context<Self>) {
        let changed = match update.change {
            SessionChange::Closed => self.model.remove_session(&update.cluster),
            SessionChange::StateChanged { .. }
            | SessionChange::ColourChanged(_)
            | SessionChange::DisplayNameChanged(_) => {
                match self.deps.sessions.get(&update.cluster) {
                    Some(session) => self.model.set_session(update.cluster, look_of(&session)),
                    None => self.model.remove_session(&update.cluster),
                }
            }
            _ => false,
        };
        // A burst of updates (a restore opening many sessions) costs one frame, not one each.
        if changed {
            cx.notify_coalesced();
        }
    }

    fn on_favourite_change(
        &mut self,
        item: Result<FavouriteChanged, FavouritesLagged>,
        cx: &mut Context<Self>,
    ) {
        match item {
            Ok(change) => {
                let name = self
                    .model
                    .favourite_name(&change.cluster)
                    .map(str::to_owned);
                self.apply_favourite(&change.cluster, name, change.favourite, cx);
            }
            // Missed some: read the catalog again.
            Err(_) => self.reload(cx),
        }
    }

    /// A favourite was marked or unmarked (here or elsewhere). `name` is what the catalog calls
    /// the cluster when it is known; otherwise the catalog is read again.
    pub(super) fn apply_favourite(
        &mut self,
        cluster: &ClusterId,
        name: Option<String>,
        favourite: bool,
        cx: &mut Context<Self>,
    ) {
        if !favourite {
            if self.model.set_favourite(cluster, "", false) {
                cx.notify();
            }
            return;
        }
        match name {
            Some(name) => {
                if self.model.set_favourite(cluster, &name, true) {
                    cx.notify();
                }
            }
            // Marked elsewhere and not known here yet: the name comes from the catalog.
            None => self.reload(cx),
        }
    }

    /// Writes the order the user just made. One write per drop.
    pub(super) fn save_order(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let order = self.model.shown_order();
        self.save = Some(cx.spawn(async move |_, cx| {
            log_failure(
                spawn_kube(&*cx, async move { store.save(&order).await }).await,
                "saving the hotbar order",
            );
        }));
    }

    /// Sends `command` through the dispatcher.
    pub(super) fn send(&self, command: Command, cx: &mut Context<Self>) {
        self.deps.dispatcher.dispatch(command, cx);
    }
}

/// Applies every item of `stream` to the view until the view is gone.
fn watch<S>(
    mut stream: S,
    cx: &mut Context<Hotbar>,
    apply: impl Fn(&mut Hotbar, S::Item, &mut Context<Hotbar>) + 'static,
) -> Task<()>
where
    S: Stream + Unpin + 'static,
    S::Item: 'static,
{
    cx.spawn(async move |this: WeakEntity<Hotbar>, cx| {
        while let Some(item) = stream.next().await {
            if this.update(cx, |this, cx| apply(this, item, cx)).is_err() {
                break;
            }
        }
    })
}

/// The value of a finished [`spawn_kube`] task; a failure (the port's, or the task's) is logged
/// as `what` and gives `None`.
fn log_failure<T>(result: Result<OxiResult<T>, KubeTaskError>, what: &str) -> Option<T> {
    match result.map_err(OxiError::from).and_then(|r| r) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(%error, "{what} failed");
            None
        }
    }
}

fn look_of(session: &ClusterSession) -> SessionLook {
    SessionLook {
        title: session.title().to_owned(),
        colour: session.colour(),
        state: session.state().clone(),
    }
}
