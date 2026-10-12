//! [`ClusterOverview`]: the workspace item, what it shows, and the clicks it sends.

use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle,
    SharedString, Task, Window,
};
use oxikube_app::WarningRow;
use oxikube_app::cluster_overview::{OverviewFeed, OverviewSnapshot};
use oxikube_app::store::ResourceStore;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_ui::IconName;
use oxikube_workspace::{Item, ItemEvent, OpenOptions, TabContent, Workspace};

use super::deps::OverviewDeps;
use super::history::HistoryPanel;
use super::info::{InfoRows, VersionState, info_rows};
use super::model::{
    CapacityLine, HealthTile, TopRow, capacity_lines, health_tiles, live_metrics_notice, top_rows,
};

/// The overview's tab key: one per cluster tab, so opening it again shows the open one.
pub const OVERVIEW_ITEM_KEY: &str = "cluster-overview";

/// How many warnings the feed shows.
pub const WARNINGS_SHOWN: usize = 8;

/// The four top lists, as rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopLists {
    /// Pods by CPU.
    pub pods_cpu: Vec<TopRow>,
    /// Pods by memory.
    pub pods_memory: Vec<TopRow>,
    /// Namespaces by CPU.
    pub namespaces_cpu: Vec<TopRow>,
    /// Namespaces by memory.
    pub namespaces_memory: Vec<TopRow>,
}

/// What the overview draws, rebuilt once per publish (never per frame).
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    /// The numbers.
    pub snapshot: Arc<OverviewSnapshot>,
    /// The health tiles.
    pub tiles: Vec<HealthTile>,
    /// CPU, memory and pods.
    pub lines: [CapacityLine; 3],
    /// The top lists.
    pub top: TopLists,
    /// `metrics unavailable: <reason>` for live usage, when it is.
    pub notice: Option<String>,
}

impl Shown {
    /// The view-model of `snapshot`; `has_metrics` says whether the app has a metrics service.
    pub fn of(snapshot: Arc<OverviewSnapshot>, has_metrics: bool) -> Self {
        let top = &snapshot.top;
        Self {
            tiles: health_tiles(&snapshot),
            lines: capacity_lines(&snapshot),
            top: TopLists {
                pods_cpu: top_rows(&top.pods_cpu, true),
                pods_memory: top_rows(&top.pods_memory, false),
                namespaces_cpu: top_rows(&top.namespaces_cpu, true),
                namespaces_memory: top_rows(&top.namespaces_memory, false),
            },
            notice: live_metrics_notice(&snapshot, has_metrics),
            snapshot,
        }
    }
}

/// While the overview is on screen: the store it follows, its feed and the tasks behind it.
/// Dropping it stops the driver (releasing the Pod and Node feeds and the metrics interests),
/// the version read, the history queries and the timers.
pub(crate) struct Live {
    pub store: ResourceStore,
    pub feed: OverviewFeed,
    pub _tasks: Vec<Task<()>>,
}

/// The cluster overview of one cluster tab (E13-S09). See the [module docs](super).
pub struct ClusterOverview {
    pub(crate) cluster: ClusterId,
    pub(crate) deps: OverviewDeps,
    pub(crate) focus: FocusHandle,
    pub(crate) scroll: ScrollHandle,
    pub(crate) shown: Shown,
    pub(crate) warnings: Vec<WarningRow>,
    pub(crate) version: VersionState,
    pub(crate) context: String,
    pub(crate) server: Option<String>,
    pub(crate) connected: bool,
    pub(crate) history: HistoryPanel,
    /// The item is the displayed tab of its pane.
    pub(crate) active: bool,
    /// The cluster's tab is the one its window shows.
    pub(crate) tab_shown: bool,
    pub(crate) live: Option<Live>,
    /// The session follower: lives as long as the item.
    pub(crate) tasks: Vec<Task<()>>,
}

impl EventEmitter<ItemEvent> for ClusterOverview {}

impl Focusable for ClusterOverview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ClusterOverview {
    /// The overview of `cluster`, following its session.
    pub fn new(cluster: ClusterId, deps: OverviewDeps, cx: &mut Context<Self>) -> Self {
        let mut this = Self::detached(cluster, deps, cx);
        this.start(cx);
        this
    }

    /// The overview of `cluster` before it follows anything.
    pub(crate) fn detached(cluster: ClusterId, deps: OverviewDeps, cx: &mut Context<Self>) -> Self {
        let has_metrics = deps.metrics.is_some();
        let history = HistoryPanel::new(&cluster, deps.history.is_some(), cx);
        Self {
            cluster,
            deps,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            shown: Shown::of(Arc::new(OverviewSnapshot::default()), has_metrics),
            warnings: Vec::new(),
            version: VersionState::Loading,
            context: String::new(),
            server: None,
            connected: false,
            history,
            active: true,
            tab_shown: true,
            live: None,
            tasks: Vec::new(),
        }
    }

    /// Shows the overview of `cluster` in `workspace`: the open one when there is one, else a new
    /// tab.
    pub fn open(
        workspace: &Entity<Workspace>,
        cluster: ClusterId,
        deps: OverviewDeps,
        window: &mut Window,
        cx: &mut App,
    ) {
        let key = SharedString::from(OVERVIEW_ITEM_KEY);
        if let Some(item) = workspace.read(cx).find_item_by_key(&key, cx) {
            workspace.update(cx, |ws, cx| ws.activate_item(item, true, window, cx));
            return;
        }
        let overview = cx.new(|cx| ClusterOverview::new(cluster, deps, cx));
        let options = OpenOptions {
            focus: true,
            reuse_existing: true,
            ..OpenOptions::default()
        };
        workspace.update(cx, |ws, cx| {
            ws.open_item_with(Box::new(overview), options, window, cx);
        });
    }

    /// The cluster.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// What the overview draws now.
    pub fn shown(&self) -> &Shown {
        &self.shown
    }

    /// The warnings the feed shows.
    pub fn warnings(&self) -> &[WarningRow] {
        &self.warnings
    }

    /// The cluster-info card's rows.
    pub fn info(&self) -> InfoRows {
        let nodes = self
            .shown
            .snapshot
            .nodes_feed
            .is_listed()
            .then(|| self.shown.snapshot.nodes.total());
        info_rows(
            &self.context,
            &self.version,
            self.shown.snapshot.distribution,
            nodes,
            self.server.as_deref(),
        )
    }

    /// The history graphs' state.
    pub fn history(&self) -> &HistoryPanel {
        &self.history
    }

    /// Whether the overview follows its feeds now (shown and connected).
    pub fn is_live(&self) -> bool {
        self.live.is_some()
    }

    /// The cluster's tab was shown (`true`) or hidden: a hidden overview releases its feeds.
    pub fn set_tab_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.tab_shown != shown {
            self.tab_shown = shown;
            self.sync(cx);
        }
    }

    /// Opens the list of `gvk` (a tile's click): `resource::OpenList` on the bus.
    pub fn open_list(&self, gvk: Gvk, cx: &mut App) {
        let cluster = self.cluster.clone();
        self.deps
            .dispatcher
            .dispatch(Command::ResourceOpenList { cluster, gvk }, cx);
    }

    /// Opens a top row's pod (`namespace` set) or namespace: `resource::Open` on the bus.
    pub fn open_row(&self, namespace: Option<&str>, name: &str, cx: &mut App) {
        let cluster = self.cluster.clone();
        let target = match namespace {
            Some(ns) => ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), ns, name),
            None => ResourceRef::cluster_scoped(cluster, Gvk::new("", "v1", "Namespace"), name),
        };
        self.deps
            .dispatcher
            .dispatch(Command::ResourceOpen { target }, cx);
    }

    /// Opens the cluster's metrics settings (`cluster::ShowMetricsSettings`): the way out of a
    /// `metrics unavailable` state.
    pub fn open_metrics_settings(&self, cx: &mut App) {
        let cluster = self.cluster.clone();
        self.deps
            .dispatcher
            .dispatch(Command::ClusterShowMetricsSettings { cluster }, cx);
    }
}

impl Item for ClusterOverview {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Cluster overview").icon(IconName::LayoutDashboard)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(OVERVIEW_ITEM_KEY.into())
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.active != active {
            self.active = active;
            self.sync(cx);
        }
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        // Release the feeds, the interests and the queries now, not when the entity drops.
        self.live = None;
        self.tasks.clear();
    }
}
