//! [`ClusterTabs`]: the controller that keeps one [`ClusterTab`] per live cluster session in the
//! window's workspace.
//!
//! | File | Holds |
//! |---|---|
//! | `mod.rs` | the entity, `start`, queries, events |
//! | `sessions.rs` | following the session manager: tabs open, refresh and close with sessions |
//! | `switch.rs` | the tab order, `cmd-1..9`, next and previous, activation |
//! | `close.rs` | closing a tab: the running-operations confirmation, then `cluster::Disconnect` |
//! | `commands.rs` | the command queue, `apply`, and the `CommandBus` registration |
//! | `persist.rs` | saving which tabs are open, in what order, which is displayed |
//! | `restore.rs` | session restore: placeholder tabs, lazy connect, the dropped-clusters notice (E06-S11) |
//!
//! # A tab per live session
//!
//! A tab exists exactly while its session is not `Disconnected` (connecting, ready, degraded,
//! needing auth, failed): the tab is where the connect lifecycle shows (E06-S06). Disconnecting
//! by any route (the tab's close button, `cluster::Disconnect`, the catalog) closes the tab, and
//! connecting opens it and displays it. The session manager stays the one authority: the
//! controller never changes a session itself, it sends `cluster::Disconnect` through the
//! dispatcher and reacts to the session updates that follow.
//!
//! # Nothing blocks
//!
//! The session updates are awaited on the GPUI executor and applied in batches by the update
//! task; opening a tab builds an empty workspace and starts its layout restore, which reads the
//! state store asynchronously. Saving is debounced and written off the UI thread.

mod close;
mod commands;
mod persist;
mod restore;
mod sessions;
mod switch;

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use futures::StreamExt as _;
use gpui::{
    App, AppContext as _, Entity, EntityId, EventEmitter, Global, Subscription, Task, WeakEntity,
    Window, WindowId,
};
use indexmap::IndexMap;
use oxikube_app::{ClusterSession, ClusterSessionManager};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::StatePort;

pub use commands::register_commands;
use persist::DebouncedSave;

use super::{
    dispatch::{CommandDispatcher, CommandSink},
    store::{ClusterTabsStore, SavedTabs},
    tab::ClusterTab,
};
use crate::persistence::MAIN_WINDOW_ID;
use crate::workspace::{Workspace, WorkspaceEvent};

/// Called when a cluster tab is created, before its layout is restored: add the cluster's
/// sidebar panel to `tab.workspace()`, register what the cluster's items need. `session` is the
/// session as it was when the tab opened.
pub type TabSetup = dyn Fn(&Entity<ClusterTab>, &ClusterSession, &mut Window, &mut App);

/// What [`ClusterTabs`] needs from the outside.
#[derive(Clone)]
pub struct ClusterTabsDeps {
    /// The sessions the tabs follow.
    pub sessions: ClusterSessionManager,
    /// Where the open tabs and each cluster's layout are saved.
    pub state: Arc<dyn StatePort>,
    /// Where `cluster::Disconnect` goes when a tab closes.
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// Populates a new tab's workspace ([`TabSetup`]); `None` leaves it empty.
    pub setup: Option<Rc<TabSetup>>,
    /// The window's id in the state store ([`MAIN_WINDOW_ID`] for the main window).
    pub window_id: String,
}

impl ClusterTabsDeps {
    /// Dependencies for the main window, with no setup hook.
    pub fn new(
        sessions: ClusterSessionManager,
        state: Arc<dyn StatePort>,
        dispatcher: Rc<dyn CommandDispatcher>,
    ) -> Self {
        Self {
            sessions,
            state,
            dispatcher,
            setup: None,
            window_id: MAIN_WINDOW_ID.to_owned(),
        }
    }

    /// Sets the hook that populates a new tab's workspace.
    #[must_use]
    pub fn with_setup(
        mut self,
        setup: impl Fn(&Entity<ClusterTab>, &ClusterSession, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.setup = Some(Rc::new(setup));
        self
    }
}

/// What the controller reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClusterTabsEvent {
    /// A tab opened for this cluster.
    Opened(ClusterId),
    /// The tab of this cluster closed.
    Closed(ClusterId),
    /// The displayed cluster changed (`None`: no cluster tab is displayed, the catalog is).
    ActiveChanged(Option<ClusterId>),
    /// The tab order changed (a tab was opened, closed or dragged).
    OrderChanged,
}

/// One open tab.
struct TabEntry {
    tab: Entity<ClusterTab>,
    /// The tab's id in the window's workspace.
    item: EntityId,
    _subscription: Subscription,
}

/// The windows' command queues, so the global key handlers find the controller of the window a
/// key was pressed in.
#[derive(Default)]
pub(super) struct TabsWindows(HashMap<WindowId, CommandSink>);

impl Global for TabsWindows {}

impl TabsWindows {
    pub(super) fn sink(cx: &App, window: WindowId) -> Option<CommandSink> {
        cx.try_global::<Self>()?.0.get(&window).cloned()
    }
}

/// See the [module docs](self).
pub struct ClusterTabs {
    workspace: WeakEntity<Workspace>,
    deps: ClusterTabsDeps,
    store: ClusterTabsStore,
    tabs: IndexMap<ClusterId, TabEntry>,
    active: Option<ClusterId>,
    sink: CommandSink,
    save: DebouncedSave,
    /// Restored clusters whose tab is shown before they connect (session restore, E06-S11).
    pending: HashSet<ClusterId>,
    /// The cluster the restore itself connects first; its tab is not connected again on display.
    restore_active: Option<ClusterId>,
    /// The restore: held here, replaced only from outside (never cleared by the task itself).
    restore_task: Option<Task<()>>,
    /// Applies session updates. Lives as long as the controller.
    _watch_sessions: Task<()>,
    /// Applies queued commands. Lives as long as the controller.
    _watch_commands: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ClusterTabsEvent> for ClusterTabs {}

impl ClusterTabs {
    /// Starts keeping `workspace` (the window's) in step with `deps.sessions`: opens a tab for
    /// every session that is not disconnected now and for every one that connects later. The
    /// workspace keeps the controller alive ([`Workspace::attach`]).
    ///
    /// # Panics
    ///
    /// When `deps.window_id` is not a valid state key (a programming error: it is a constant).
    pub fn start(
        workspace: &Entity<Workspace>,
        deps: ClusterTabsDeps,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let store = ClusterTabsStore::new(deps.state.clone(), &deps.window_id)
            .expect("the window id is a valid state key");
        let (sink, mut commands) = CommandSink::channel();
        let window_id = window.window_handle().window_id();
        cx.default_global::<TabsWindows>()
            .0
            .insert(window_id, sink.clone());

        let this = cx.new(|cx| {
            // Subscribe before reading the sessions, so no update falls between the two.
            let mut updates = deps.sessions.subscribe();
            let watch_sessions = cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
                while let Some(item) = updates.next().await {
                    let alive = this.update_in(cx, |this, window, cx| match item {
                        Ok(update) => this.apply_session_update(update, window, cx),
                        // Missed some: re-read every session instead of replaying.
                        Err(_) => this.sync_sessions(window, cx),
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            });
            let watch_commands = cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
                while let Some(command) = commands.next().await {
                    let alive = this.update_in(cx, |this, window, cx| {
                        this.apply(&command, window, cx);
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            });
            let subscriptions = vec![
                // Dragging a tab reorders the cluster tabs: save the new order.
                cx.subscribe(workspace, |this, _, _: &WorkspaceEvent, cx| {
                    this.layout_changed(cx)
                }),
                cx.on_app_quit(|this: &mut Self, cx| this.on_quit(cx)),
                cx.on_release(move |_, cx| {
                    cx.default_global::<TabsWindows>().0.remove(&window_id);
                }),
            ];
            Self {
                workspace: workspace.downgrade(),
                deps,
                store,
                tabs: IndexMap::new(),
                active: None,
                sink,
                save: DebouncedSave::default(),
                pending: HashSet::new(),
                restore_active: None,
                restore_task: None,
                _watch_sessions: watch_sessions,
                _watch_commands: watch_commands,
                _subscriptions: subscriptions,
            }
        });
        this.update(cx, |this, cx| this.sync_sessions(window, cx));
        workspace.update(cx, |ws, _| ws.attach(this.clone()));
        this
    }

    /// The queue the `CommandBus` handlers ([`register_commands`]) and [`TabsDispatcher`](
    /// super::TabsDispatcher) push the tab commands into.
    pub fn command_sink(&self) -> CommandSink {
        self.sink.clone()
    }

    /// The store the open tabs are saved in; session restore (E06-S11) reads it through the same
    /// store.
    pub fn store(&self) -> &ClusterTabsStore {
        &self.store
    }

    /// The clusters with an open tab, in the order the tabs are shown.
    pub fn clusters(&self, cx: &App) -> Vec<ClusterId> {
        self.display_order(cx)
    }

    /// The tab of `cluster`.
    pub fn tab(&self, cluster: &ClusterId) -> Option<&Entity<ClusterTab>> {
        self.tabs.get(cluster).map(|entry| &entry.tab)
    }

    /// The cluster whose tab is displayed, `None` when the catalog (or another non-cluster
    /// item) is.
    pub fn active(&self) -> Option<&ClusterId> {
        self.active.as_ref()
    }

    /// How many cluster tabs are open.
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// Whether no cluster tab is open.
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// What would be saved now.
    pub fn snapshot(&self, cx: &App) -> SavedTabs {
        let order = self.display_order(cx);
        let titles = order.iter().filter_map(|cluster| {
            let tab = self.tabs.get(cluster)?.tab.read(cx);
            Some((cluster.clone(), tab.info().title.to_string()))
        });
        let titles: Vec<_> = titles.collect();
        SavedTabs::new(order, self.active.clone()).with_titles(titles)
    }
}
