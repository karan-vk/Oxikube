//! [`WorkloadsOverview`]: the workspace item that draws the tiles.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Task, Window, div, px,
};
use oxikube_app::{
    ClusterSessionManager, CountState, RecentWarnings, ResourceStores, WarningRow, WorkloadCounters,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::view::Bucket;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, IconName, u};
use oxikube_workspace::{
    CommandDispatcher, Item, ItemEvent, ItemHandle, OpenOptions, TabContent, Workspace,
};

use super::tiles::{Tile, TileRegistry};

/// The overview's tab key: one overview per cluster tab, so opening it again shows the open one.
pub const OVERVIEW_ITEM_KEY: &str = "overview";

/// What the overview is built from.
#[derive(Clone)]
pub struct OverviewDeps {
    /// The sessions: the cluster's selection and connection, and their updates.
    pub sessions: ClusterSessionManager,
    /// The per-cluster stores the tiles read.
    pub stores: Arc<ResourceStores>,
    /// Where a tile click sends its command (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The newest warning events for the warnings panel: the Events feed (E14) once it exists,
    /// else [`NoRecentWarnings`](oxikube_app::NoRecentWarnings).
    pub warnings: Arc<dyn RecentWarnings>,
}

/// The Workloads overview of one cluster. See the [module docs](super).
pub struct WorkloadsOverview {
    pub(super) cluster: ClusterId,
    pub(super) deps: OverviewDeps,
    focus: FocusHandle,
    pub(super) tiles: Vec<Tile>,
    /// The tiles' feeds and their counts, in tile order.
    pub(super) counters: WorkloadCounters,
    /// The warnings panel's rows, newest first.
    pub(super) warnings: Vec<WarningRow>,
    /// The refresh timer and the session follower. Live as long as the item; nothing clears
    /// them from inside.
    pub(super) tasks: Vec<Task<()>>,
    pub(super) subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<ItemEvent> for WorkloadsOverview {}

impl Focusable for WorkloadsOverview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl WorkloadsOverview {
    /// The overview of `cluster`, following its session.
    pub fn new(cluster: ClusterId, deps: OverviewDeps, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            cluster,
            deps,
            focus: cx.focus_handle(),
            tiles: Vec::new(),
            counters: WorkloadCounters::new(Vec::new()),
            warnings: Vec::new(),
            tasks: Vec::new(),
            subscriptions: Vec::new(),
        };
        this.start(cx);
        this
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
        if let Some(open) = workspace.read(cx).find_item_by_key(&key, cx) {
            workspace.update(cx, |ws, cx| ws.activate_item(open, true, window, cx));
            return;
        }
        let item: Box<dyn ItemHandle> =
            Box::new(cx.new(|cx| WorkloadsOverview::new(cluster, deps, cx)));
        workspace.update(cx, |ws, cx| {
            ws.open_item_with(item, OpenOptions::default(), window, cx);
        });
    }

    /// The tiles with their current answers, in order.
    pub fn tiles(&self) -> impl Iterator<Item = (&Tile, &CountState)> {
        self.tiles.iter().zip(self.counters.states())
    }

    /// The answer for the tile `id`.
    pub fn state_of(&self, id: &str) -> Option<&CountState> {
        self.tiles()
            .find(|(tile, _)| &*tile.id == id)
            .map(|(_, state)| state)
    }

    /// Opens the list of the tile `id`'s kind: sends `resource::OpenList` on the bus.
    pub fn open_tile(&self, id: &str, cx: &mut App) {
        let Some(tile) = self.tiles.iter().find(|t| &*t.id == id) else {
            return;
        };
        self.deps.dispatcher.dispatch(
            Command::ResourceOpenList {
                cluster: self.cluster.clone(),
                gvk: tile.target.gvk.clone(),
            },
            cx,
        );
    }

    /// Opens the list of the tile `id`'s kind filtered to `bucket` (`-s <bucket>`): sends
    /// `resource::OpenList` and `table::SetFilter` on the bus, the pair the jump bar sends for
    /// `:pod /-s pending`. The filter waits for the table when it is not open yet.
    pub fn open_bucket(&self, id: &str, bucket: Bucket, cx: &mut App) {
        let Some(tile) = self.tiles.iter().find(|t| &*t.id == id) else {
            return;
        };
        let (cluster, gvk) = (self.cluster.clone(), tile.target.gvk.clone());
        self.deps.dispatcher.dispatch(
            Command::ResourceOpenList {
                cluster: cluster.clone(),
                gvk: gvk.clone(),
            },
            cx,
        );
        self.deps.dispatcher.dispatch(
            Command::TableSetFilter {
                cluster,
                gvk,
                text: format!("-s {}", bucket.word()),
            },
            cx,
        );
    }

    /// The rows the warnings panel shows now.
    pub fn warnings(&self) -> &[WarningRow] {
        &self.warnings
    }

    /// Replaces the tiles from the registry; the feeds follow in `sync`.
    pub(super) fn reload_tiles(&mut self, cx: &App) {
        self.tiles = TileRegistry::tiles(cx);
        let targets = self.tiles.iter().map(|t| t.target.clone()).collect();
        self.counters.set_targets(targets);
    }
}

impl Item for WorkloadsOverview {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Overview").icon(IconName::LayoutDashboard)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(OVERVIEW_ITEM_KEY.into())
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        // Release the feeds now rather than when the entity is dropped.
        self.counters.release();
        self.tasks.clear();
    }
}

impl Render for WorkloadsOverview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let connected = self.counters.is_open();
        let cards: Vec<_> = self
            .tiles
            .iter()
            .zip(self.counters.states())
            .map(|(tile, state)| self.card(tile, state, cx))
            .collect();
        v_flex()
            .id("workloads-overview")
            .debug_selector(|| "workloads-overview".to_owned())
            .key_context("Overview")
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .bg(colors.background)
            .text_color(colors.text)
            .p(u(tokens.spacing.xl))
            .gap(u(tokens.spacing.lg))
            .child(
                div()
                    .text_size(u(tokens.font.heading))
                    .font_semibold()
                    .child("Workloads"),
            )
            .child(if connected {
                v_flex()
                    .gap(u(tokens.spacing.lg))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .items_start()
                            .gap(u(tokens.spacing.lg))
                            .children(cards),
                    )
                    .child(self.warnings_panel(cx))
                    .into_any_element()
            } else {
                div()
                    .debug_selector(|| "overview-not-connected".to_owned())
                    .text_size(u(tokens.font.body))
                    .text_color(colors.text_muted)
                    .child("Not connected")
                    .into_any_element()
            })
            .child(div().h(u(px(1.))))
    }
}
