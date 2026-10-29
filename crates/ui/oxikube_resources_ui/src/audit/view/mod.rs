//! [`AuditView`]: the audit log in a virtualised table with filters, a detail panel and an
//! export. A workspace item of the cluster's tab (one per tab, [`ITEM_KEY`]).
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | the view, its deps, opening it, the item glue |
//! | `load` | the first page, older pages, the newest page after an [`AuditAppended`](oxikube_app::AuditAppended), the "N new" pill |
//! | `filters` | the filter bar and what changing a filter does |
//! | `detail` | the side panel: every field of a record, its batch, "Show resource" |
//! | `export` | `audit::Export`: the save panel and the streamed write |
//! | `render` | the toolbar, the table, the states and the status line |
//! | `delegate` | the table's data source |

mod delegate;
mod detail;
mod export;
mod filters;
pub(super) mod load;
mod render;

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Task, WeakEntity, Window,
};
use oxikube_app::audit::query::{AuditFilters, AuditReader, NewRecords};
use oxikube_app::{AuditListener, ClusterSessionManager};
use oxikube_domain::audit::AuditRecord;
use oxikube_ports::{ClockPort, FsPort};
use oxikube_ui::IconName;
use oxikube_ui::table::{TableEvent, TableHandle, TableOptions};
use oxikube_workspace::{
    CommandDispatcher, Item, ItemEvent, ItemHandle, OpenOptions, TabContent, Workspace,
};

pub use delegate::AuditDelegate;

use super::model::{AuditModel, ClusterNames};

/// The viewer's tab key: one per cluster tab, so opening it again shows the open one.
pub const ITEM_KEY: &str = "audit";

/// What every audit viewer is built over.
#[derive(Clone)]
pub struct AuditViewDeps {
    /// Reads the audit log.
    pub reader: AuditReader,
    /// Tells the viewer that records were stored.
    pub listener: AuditListener,
    /// The sessions (the context names of the cluster cells and the cluster filter).
    pub sessions: ClusterSessionManager,
    /// Where the viewer sends its commands (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// Where an export is written.
    pub fs: Arc<dyn FsPort>,
    /// The clock of the time-window filter and the export's file name.
    pub clock: Arc<dyn ClockPort>,
}

/// Where the load stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Phase {
    /// The first page is being read.
    Loading,
    /// The first page arrived.
    Ready,
}

/// The records of the selected record's batch.
#[derive(Debug, Clone)]
pub(crate) enum BatchLoad {
    Loading,
    Ready(Vec<AuditRecord>),
    Failed(SharedString),
}

/// The batch the detail panel lists, for the batch id it was asked for.
#[derive(Debug, Clone)]
pub(crate) struct BatchState {
    pub(crate) id: Arc<str>,
    pub(crate) load: BatchLoad,
}

/// The audit log viewer. See the [module docs](self).
pub struct AuditView {
    pub(crate) deps: AuditViewDeps,
    /// The workspace the view is a tab of: where its toasts show.
    pub(crate) workspace: Option<WeakEntity<Workspace>>,
    pub(crate) focus: FocusHandle,
    pub(crate) table: TableHandle<AuditDelegate>,
    pub(crate) filters: AuditFilters,
    pub(crate) phase: Phase,
    /// Why the last read failed, while it did: the database is unavailable.
    pub(crate) error: Option<SharedString>,
    /// Records newer than the head that are not shown yet (the "N new" pill).
    pub(crate) pending_new: Option<NewRecords>,
    /// The first visible row, as last reported.
    pub(crate) first_visible: usize,
    /// Names the newest filter: a page read for an older one that lands late is dropped.
    pub(crate) generation: u64,
    /// The newest page is being read (a signal that arrives meanwhile waits in `head_dirty`).
    pub(crate) head_busy: bool,
    pub(crate) head_dirty: bool,
    pub(crate) batch: Option<BatchState>,
    pub(crate) first_task: Option<Task<()>>,
    pub(crate) more_task: Option<Task<()>>,
    pub(crate) head_task: Option<Task<()>>,
    pub(crate) batch_task: Option<Task<()>>,
    pub(crate) export_task: Option<Task<()>>,
    pub(crate) live_task: Option<Task<()>>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ItemEvent> for AuditView {}

impl Focusable for AuditView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl AuditView {
    /// A viewer over `deps`; it reads the newest page and listens for new records at once.
    pub fn new(
        deps: AuditViewDeps,
        workspace: Option<WeakEntity<Workspace>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let delegate = AuditDelegate::new(cx.entity().downgrade());
        let options = TableOptions {
            sortable: false,
            resizable_columns: true,
            movable_columns: false,
            loop_selection: false,
            select_rows: false,
        };
        let table = TableHandle::with_options(delegate, options, window, cx);
        let mut this = Self {
            deps,
            workspace,
            focus: cx.focus_handle(),
            table,
            filters: AuditFilters::default(),
            phase: Phase::Loading,
            error: None,
            pending_new: None,
            first_visible: 0,
            generation: 0,
            head_busy: false,
            head_dirty: false,
            batch: None,
            first_task: None,
            more_task: None,
            head_task: None,
            batch_task: None,
            export_task: None,
            live_task: None,
            _subscriptions: Vec::new(),
        };
        this.wire(window, cx);
        this
    }

    /// Shows the viewer in `workspace`: the open one when there is one, else a new tab.
    pub fn open(
        workspace: &Entity<Workspace>,
        deps: AuditViewDeps,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<AuditView> {
        if let Some(open) = Self::find(workspace, cx) {
            let id = open.entity_id();
            workspace.update(cx, |ws, cx| ws.activate_item(id, true, window, cx));
            return open;
        }
        let weak = workspace.downgrade();
        let view = cx.new(|cx| AuditView::new(deps, Some(weak), window, cx));
        let item: Box<dyn ItemHandle> = Box::new(view.clone());
        let options = OpenOptions {
            focus: true,
            reuse_existing: true,
            ..OpenOptions::default()
        };
        workspace.update(cx, |ws, cx| ws.open_item_with(item, options, window, cx));
        view
    }

    /// The viewer open in `workspace`, if any.
    pub fn find(workspace: &Entity<Workspace>, cx: &App) -> Option<Entity<AuditView>> {
        workspace
            .read(cx)
            .items_of_type::<AuditView>()
            .into_iter()
            .next()
    }

    /// The loaded rows, the cursor and the selection.
    pub fn model<'a>(&self, cx: &'a App) -> &'a AuditModel {
        &self.table.delegate(cx).model
    }

    /// The rows the table shows now (the perf scenario drives its scroll by them).
    pub fn visible_rows(&self, cx: &App) -> std::ops::Range<usize> {
        self.table.visible_rows(cx)
    }

    /// What the filter bar says.
    pub fn filters(&self) -> &AuditFilters {
        &self.filters
    }

    /// The "N new" pill's count, while there are records the viewer does not show yet.
    pub fn pending_new(&self) -> Option<NewRecords> {
        self.pending_new
    }

    /// Why the audit log could not be read, while it could not.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Puts the focus on the rows.
    pub fn focus_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    /// Shows `toast` in the workspace the view is a tab of; nothing without one.
    pub(crate) fn toast(&self, toast: oxikube_workspace::Toast, cx: &mut Context<Self>) {
        if let Some(workspace) = &self.workspace {
            workspace
                .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                .ok();
        }
    }

    /// The names of the clusters the sessions know.
    pub(crate) fn cluster_names(&self) -> ClusterNames {
        ClusterNames::new(self.deps.sessions.sessions().into_iter().map(|session| {
            (
                session.id().clone(),
                SharedString::from(session.context().to_string()),
            )
        }))
    }

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
        self.start_live(cx);
        self.reload(cx);
    }

    fn on_table_event(&mut self, event: &TableEvent, cx: &mut Context<Self>) {
        if let TableEvent::RowClicked(click) = event {
            self.select(Some(click.row), cx);
        }
    }
}

impl Item for AuditView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Audit log").icon(IconName::FileText)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        // Stop listening and reading now rather than when the entity is dropped.
        self.live_task = None;
        self.first_task = None;
        self.more_task = None;
        self.head_task = None;
        self.batch_task = None;
    }
}

/// `button` in a box findable as `selector` in a test window.
pub(crate) fn tagged(
    selector: &'static str,
    button: oxikube_ui::button::Button,
) -> gpui::AnyElement {
    use gpui::{InteractiveElement as _, IntoElement as _, ParentElement as _, Styled as _};
    gpui::div()
        .flex_none()
        .debug_selector(move || selector.to_owned())
        .child(button)
        .into_any_element()
}
