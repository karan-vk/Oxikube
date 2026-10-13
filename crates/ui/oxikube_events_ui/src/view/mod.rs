//! [`EventsView`]: a cluster's events, live, in a virtualised table with a filter toolbar. A
//! workspace item of the cluster's tab (one per tab, [`ITEM_KEY`]).
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | the view, its deps, opening it, the item and command-surface glue |
//! | `live` | the subscription to the `EventService`, the lists read off the UI thread, the tail's pause, the clock of the relative cells |
//! | `filters` | the toolbar's fields and toggles, compiling and saving the filters |
//! | `keys` | the key actions and what they dispatch: selection, open, copy, export |
//! | `render` | the toolbar, the table and the status line |
//! | `delegate` | the table's data source |
//! | `actions` | the key actions of the `EventsView` context |
//!
//! # Data, and what it costs
//!
//! The view never watches the cluster: it subscribes to the app's `EventService` (one events
//! feed per cluster, which the cluster's tab keeps running) with the compiled filter. The
//! subscription's first item and every filter change are a reset: the matching events are read
//! and turned into rows on the background executor and swapped in. Between resets each coalesced
//! change (at most one per poll, so per frame) is placed with a binary search, and the redraw is
//! [`notify_coalesced`](oxikube_runtime::notify_coalesced). Input (typing, toggles, keys) redraws
//! at once.

mod actions;
mod delegate;
mod filters;
mod keys;
mod live;
mod render;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Task, WeakEntity, Window,
};
use oxikube_app::events::{
    EventFilter, EventService, EventsChanged, EventsState, EventsSubscription,
};
use oxikube_app::{ClusterSessionManager, CommandTarget};
use oxikube_domain::command::ViewContext;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{ClockPort, FsPort, StatePort};
use oxikube_ui::IconName;
use oxikube_ui::input::InputState;
use oxikube_ui::table::{TableHandle, TableOptions};
use oxikube_workspace::command_surface::{self, CommandSurface};
use oxikube_workspace::{
    CommandDispatcher, Item, ItemEvent, ItemHandle, OpenOptions, TabContent, Workspace,
};

pub use actions::{
    CopyRow, CopyRowJson, ExportJson, FocusFilter, FocusTable, OpenSelected, SelectFirst,
    SelectLast, SelectNext, SelectPrevious, ToggleFaults, TogglePause,
};
pub use delegate::EventsDelegate;
pub use filters::Field;

use crate::filter::EventsFilters;
use crate::model::EventsModel;
use crate::persist::FiltersWriter;

/// The view's tab key: one per cluster tab, so opening it again shows the open one.
pub const ITEM_KEY: &str = "events";

/// What every events view is built over.
#[derive(Clone)]
pub struct EventsViewDeps {
    /// The app's events: the one feed per cluster the view reads.
    pub service: EventService,
    /// The sessions (the context's name for an export's file name).
    pub sessions: ClusterSessionManager,
    /// Where the view sends its commands (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// Where the filters are saved per cluster.
    pub state: Arc<dyn StatePort>,
    /// Where an export is written.
    pub fs: Arc<dyn FsPort>,
    /// The clock of the Age and Last seen cells.
    pub clock: Arc<dyn ClockPort>,
}

/// The four text fields of the toolbar.
pub(crate) struct Fields {
    pub(crate) reason: Entity<InputState>,
    pub(crate) kind: Entity<InputState>,
    pub(crate) namespace: Entity<InputState>,
    pub(crate) text: Entity<InputState>,
}

/// A cluster's events. See the [module docs](self).
pub struct EventsView {
    pub(crate) cluster: ClusterId,
    pub(crate) deps: EventsViewDeps,
    /// The workspace the view is a tab of: where its toasts show.
    pub(crate) workspace: Option<WeakEntity<Workspace>>,
    pub(crate) focus: FocusHandle,
    pub(crate) table: TableHandle<EventsDelegate>,
    pub(crate) fields: Fields,
    /// What the toolbar says, as typed.
    pub(crate) filters: EventsFilters,
    /// The filter the rows are for: the last good compile of `filters`.
    pub(crate) applied: EventFilter,
    /// Why the text is not a filter, while it is not.
    pub(crate) filter_error: Option<SharedString>,
    /// Whether a filter field has the focus (bare keys are text then).
    pub(crate) editing: bool,
    /// The saved filters arrived (or there were none): edits are saved from then on.
    pub(crate) loaded: bool,
    /// The filter changed since the last reset: the next reset is its rows, shown even while the
    /// tail is paused.
    pub(crate) refiltered: bool,
    pub(crate) writer: Option<FiltersWriter>,
    /// The subscription; the live task polls it, the view changes its filter.
    pub(crate) subscription: Rc<RefCell<EventsSubscription>>,
    /// The feed's state, as last told.
    pub(crate) feed: EventsState,
    /// A list being read off the UI thread: changes wait in `held` until it lands.
    pub(crate) reading: bool,
    pub(crate) held: Vec<EventsChanged>,
    /// Names the newest read; an older one that lands late is dropped.
    pub(crate) generation: u64,
    /// Whether the view is the displayed tab (the relative cells tick only then).
    pub(crate) active: bool,
    pub(crate) read_task: Option<Task<()>>,
    pub(crate) export_task: Option<Task<()>>,
    pub(crate) tasks: Vec<Task<()>>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ItemEvent> for EventsView {}

impl Focusable for EventsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventsView {
    /// The events of `cluster`, live. The saved filters of the cluster are read and applied as
    /// soon as they arrive.
    pub fn new(
        cluster: ClusterId,
        deps: EventsViewDeps,
        workspace: Option<WeakEntity<Workspace>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let now = deps.clock.now();
        let delegate = EventsDelegate::new(cx.entity().downgrade(), now);
        let options = TableOptions {
            sortable: true,
            resizable_columns: true,
            movable_columns: false,
            loop_selection: false,
            select_rows: false,
        };
        let table = TableHandle::with_options(delegate, options, window, cx);
        let focus = cx.focus_handle();
        let fields = filters::fields(window, cx);
        let subscription = deps.service.subscribe(&cluster, EventFilter::new());
        // The feed may be running already (the cluster's tab subscribes first): its state now.
        let feed = subscription.state();
        let mut this = Self {
            cluster,
            deps,
            workspace,
            focus,
            table,
            fields,
            filters: EventsFilters::default(),
            applied: EventFilter::new(),
            filter_error: None,
            editing: false,
            loaded: false,
            refiltered: false,
            writer: None,
            subscription: Rc::new(RefCell::new(subscription)),
            feed,
            reading: false,
            held: Vec::new(),
            generation: 0,
            active: true,
            read_task: None,
            export_task: None,
            tasks: Vec::new(),
            _subscriptions: Vec::new(),
        };
        this.wire(window, cx);
        this
    }

    /// Shows the events of `cluster` in `workspace`: the open view when there is one, else a new
    /// tab. Returns the view.
    pub fn open(
        workspace: &Entity<Workspace>,
        cluster: ClusterId,
        deps: EventsViewDeps,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<EventsView> {
        if let Some(open) = Self::find(workspace, cx) {
            let id = open.entity_id();
            workspace.update(cx, |ws, cx| ws.activate_item(id, true, window, cx));
            return open;
        }
        let weak = workspace.downgrade();
        let view = cx.new(|cx| EventsView::new(cluster, deps, Some(weak), window, cx));
        let item: Box<dyn ItemHandle> = Box::new(view.clone());
        let options = OpenOptions {
            focus: true,
            reuse_existing: true,
            ..OpenOptions::default()
        };
        workspace.update(cx, |ws, cx| ws.open_item_with(item, options, window, cx));
        view
    }

    /// The events view open in `workspace`, if any.
    pub fn find(workspace: &Entity<Workspace>, cx: &App) -> Option<Entity<EventsView>> {
        workspace
            .read(cx)
            .items_of_type::<EventsView>()
            .into_iter()
            .next()
    }

    /// The cluster the view shows.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// The rows, the selection and the tail.
    pub fn model<'a>(&self, cx: &'a App) -> &'a EventsModel {
        &self.table.delegate(cx).model
    }

    /// What the toolbar says.
    pub fn filters(&self) -> &EventsFilters {
        &self.filters
    }

    /// Why the text filter is not a filter, while it is not.
    pub fn filter_error(&self) -> Option<&str> {
        self.filter_error.as_deref()
    }

    /// The feed's state, as last told.
    pub fn feed_state(&self) -> &EventsState {
        &self.feed
    }

    /// Wires the toolbar, the table, the live task and the saved filters.
    fn wire(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let events = self.table.on_event(cx, move |event, cx| {
            weak.update(cx, |view, cx| view.on_table_event(event, cx))
                .ok();
        });
        // Clicking a header or a row focuses the library's table; the keys live on this view.
        let refocus = cx.on_focus(&self.table.focus_handle(cx), window, |view, window, cx| {
            window.focus(&view.focus, cx);
        });
        self._subscriptions.push(events);
        self._subscriptions.push(refocus);
        self.subscribe_fields(window, cx);
        self.start_live(cx);
        self.start_clock(cx);
        self.load_saved(window, cx);
        command_surface::register(&cx.entity(), &self.focus, cx);
    }
}

impl CommandSurface for EventsView {
    fn view_context(&self) -> ViewContext {
        ViewContext::Events
    }

    fn command_target(&self, _: &App) -> CommandTarget {
        CommandTarget::none().in_cluster(self.cluster.clone())
    }
}

impl Item for EventsView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Events").icon(IconName::Bell)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.active != active {
            self.active = active;
            if active {
                self.tick(cx);
            }
        }
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        // Stop polling and reading now rather than when the entity is dropped.
        self.tasks.clear();
        self.read_task = None;
    }
}

#[cfg(test)]
mod tests;
