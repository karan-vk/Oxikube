//! [`PulsesView`]: the workspace item that draws the page and handles its keys.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, px,
};
use oxikube_app::{
    ClusterSessionManager, CountsLease, MetricsInterest, MetricsService, ResourceStore,
    ResourceStores,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_keymap::{KeyContextBuilder, contexts};
use oxikube_ports::ClockPort;
use oxikube_ui::layout::v_flex;
use oxikube_ui::{ActiveTokens as _, IconName, u};
use oxikube_workspace::{
    CommandDispatcher, Item, ItemEvent, ItemHandle, OpenOptions, TabContent, Workspace,
};

use super::actions::{
    Close, Open, Refresh, SelectDown, SelectFirst, SelectLast, SelectLeft, SelectRight, SelectUp,
};
use super::fallback::Recount;
use super::grid;
use super::model::{Move, PulsesModel};
use super::specs::COLUMNS;

/// The page's tab key: one per cluster tab, so opening it again shows the open one.
pub const ITEM_KEY: &str = "pulses";

/// What the page is built from.
#[derive(Clone)]
pub struct PulsesDeps {
    /// The sessions: the cluster's selection and connection, and their updates.
    pub sessions: ClusterSessionManager,
    /// The per-cluster stores the tiles count from.
    pub stores: Arc<ResourceStores>,
    /// Where a drill-down sends its command (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The live metrics service behind the CPU and memory gauges. `None`: the gauges say there
    /// are no metrics.
    pub metrics: Option<MetricsService>,
    /// The clock the history is timed on.
    pub clock: Arc<dyn ClockPort>,
}

/// The pulses of one cluster. See the [module docs](super).
pub struct PulsesView {
    pub(super) cluster: ClusterId,
    pub(super) deps: PulsesDeps,
    pub(super) focus: FocusHandle,
    pub(super) scroll: ScrollHandle,
    pub(super) model: PulsesModel,
    pub(super) store: Option<ResourceStore>,
    pub(super) lease: Option<CountsLease>,
    /// Held while the page is the displayed tab: the Nodes poll runs only then.
    pub(super) interest: Option<MetricsInterest>,
    /// Whether the page is the displayed tab of its pane. A page nobody sees counts and polls
    /// nothing.
    pub(super) active: bool,
    /// Ticks since the lease was made: a kind not watched after the first few is really not.
    pub(super) age: u32,
    /// The namespace selection the lease and the one-off counts are scoped to.
    pub(super) scoped_to: Option<NamespaceSelection>,
    /// Seconds (clock) of the last history sample.
    pub(super) last_sample: Option<f64>,
    /// The one-off count of each tile, by tile.
    pub(super) recounts: Vec<Recount>,
    /// The cluster gauges' computation, replaced by the next one (dropping it cancels it).
    pub(super) gauge_task: Option<Task<()>>,
    /// What the gauges were last computed from: the usage epoch and the node count.
    pub(super) gauge_stamp: Option<(u64, usize)>,
    /// The timer, the session follower and the metrics follower. Live as long as the item.
    pub(super) tasks: Vec<Task<()>>,
}

impl EventEmitter<ItemEvent> for PulsesView {}

impl Focusable for PulsesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PulsesView {
    /// The pulses of `cluster`, following its session.
    pub fn new(cluster: ClusterId, deps: PulsesDeps, cx: &mut Context<Self>) -> Self {
        let model = PulsesModel::new();
        let tiles = model.tiles().len();
        let mut this = Self {
            cluster,
            deps,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            model,
            store: None,
            lease: None,
            interest: None,
            active: true,
            age: 0,
            scoped_to: None,
            last_sample: None,
            recounts: (0..tiles).map(|_| Recount::default()).collect(),
            gauge_task: None,
            gauge_stamp: None,
            tasks: Vec::new(),
        };
        this.start(cx);
        this
    }

    /// Shows the pulses of `cluster` in `workspace`: the open page when there is one, else a new
    /// tab.
    pub fn open(
        workspace: &Entity<Workspace>,
        cluster: ClusterId,
        deps: PulsesDeps,
        window: &mut Window,
        cx: &mut App,
    ) {
        let key = SharedString::from(ITEM_KEY);
        if let Some(open) = workspace.read(cx).find_item_by_key(&key, cx) {
            workspace.update(cx, |ws, cx| ws.activate_item(open, true, window, cx));
            return;
        }
        let item: Box<dyn ItemHandle> = Box::new(cx.new(|cx| PulsesView::new(cluster, deps, cx)));
        let options = OpenOptions {
            focus: true,
            reuse_existing: true,
            ..OpenOptions::default()
        };
        workspace.update(cx, |ws, cx| ws.open_item_with(item, options, window, cx));
    }

    /// What the page holds.
    pub fn model(&self) -> &PulsesModel {
        &self.model
    }

    /// The cluster the page shows.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// Moves the selection one step; scrolls the row into view.
    pub fn step(&mut self, movement: Move, cx: &mut Context<Self>) {
        if self.model.step(movement) {
            self.scroll.scroll_to_item(self.model.selected() / COLUMNS);
            cx.notify();
        }
    }

    /// Selects the tile `index` and opens its table (a click).
    pub fn click(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.model.select(index) {
            cx.notify();
        }
        self.open_selected(cx);
    }

    /// Opens the table of the selected tile's kind: `resource::OpenList` on the bus. The
    /// namespaces the session selected apply to it, as they do to the counts.
    pub fn open_selected(&self, cx: &mut App) {
        let gvk = self.model.selected_tile().spec.drill.clone();
        self.deps.dispatcher.dispatch(
            Command::ResourceOpenList {
                cluster: self.cluster.clone(),
                gvk,
            },
            cx,
        );
    }

    fn key_context(&self) -> gpui::KeyContext {
        KeyContextBuilder::new(contexts::PULSES).build()
    }
}

impl Item for PulsesView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Pulses").icon(IconName::Activity)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active;
        self.sync(cx);
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        // Release the feeds and the poll now rather than when the entity is dropped.
        self.release();
        self.tasks.clear();
    }
}

impl Render for PulsesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let connected = self.store.is_some();
        let rows = if connected {
            let entity = cx.entity();
            grid::rows(&mut self.model, &tokens, move |index| {
                let entity = entity.clone();
                Some(Box::new(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.click(index, cx));
                }))
            })
        } else {
            Vec::new()
        };
        v_flex()
            .id("pulses")
            .debug_selector(|| "pulses".to_owned())
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.step(Move::Left, cx)))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.step(Move::Right, cx)))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.step(Move::Up, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.step(Move::Down, cx)))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.step(Move::First, cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.step(Move::Last, cx)))
            .on_action(cx.listener(|this, _: &Open, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.recount(cx)))
            .on_action(cx.listener(|_, _: &Close, _, cx| cx.emit(ItemEvent::CloseItem)))
            .size_full()
            .overflow_hidden()
            .bg(colors.background)
            .text_color(colors.text)
            .p(u(tokens.spacing.xl))
            .gap(u(tokens.spacing.lg))
            .child(grid::header(&tokens))
            .child(if connected {
                v_flex()
                    .id("pulses-grid")
                    .debug_selector(|| "pulses-grid".to_owned())
                    .track_scroll(&self.scroll)
                    .overflow_y_scroll()
                    .flex_1()
                    .gap(u(tokens.spacing.lg))
                    .children(rows)
                    .into_any_element()
            } else {
                div()
                    .debug_selector(|| "pulses-not-connected".to_owned())
                    .text_size(u(tokens.font.body))
                    .text_color(colors.text_muted)
                    .child("Not connected")
                    .into_any_element()
            })
            .child(div().h(u(px(1.))))
    }
}
