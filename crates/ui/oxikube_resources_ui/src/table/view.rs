//! [`ResourceTable`]: the list of one kind in one cluster, as a workspace item: the entity, its
//! construction and the `Item` impl. See the [`table`](crate::table) module docs for the files.
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Task, Window,
};
use jiff::Timestamp;
use oxikube_app::store::{
    FeedKind, FeedState, FilterParts, ObjectKey, ResourceStore, ResourceStores,
};
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::kinds::ResourceKind;
use oxikube_domain::session::WatchScope;
use oxikube_keymap::{KeyContextBuilder, KeyContextual, contexts};
use oxikube_ports::{StatePort, TableSource};
use oxikube_runtime::RenderGate;
use oxikube_ui::TableHandle;
use oxikube_ui::table::TableEvent;
use oxikube_ui::table::TableOptions;
use oxikube_workspace::{CommandDispatcher, Item, ItemEvent, TabContent, Workspace};

use super::cell_cache::CellCache;
use super::columns::{ColumnScope, initial_provider};
use super::delegate::RowsDelegate;
use super::layout::ColumnLayout;
use super::prefs::{ColumnPrefs, PrefsWriter};
use super::selection::Selection;
use super::states::{StateLabels, scope_label};
use crate::actions::{ActionSource, ResourceActions};
use crate::filter::{FilterBar, FilterWriter};

/// How often the table checks whether a visible age moved while it is shown. The check redraws
/// only when one did (see [`CellCache::ages_moved`]), so a still table is not redrawn at this rate.
pub(super) const TICK: Duration = Duration::from_secs(1);

/// What a [`ResourceTable`] is built over. Cheap to clone.
#[derive(Clone)]
pub struct ResourceTableDeps {
    /// The sessions: the cluster's connection, namespace selection and capabilities.
    pub sessions: ClusterSessionManager,
    /// The per-session resource stores the table subscribes to.
    pub stores: Arc<ResourceStores>,
    /// The column catalogue of the core kinds.
    pub columns: Arc<CoreColumns>,
    /// Where column layouts are saved.
    pub state: Arc<dyn StatePort>,
    /// Where the table's commands go (`resource::Open`, `CopyName`, `SelectAll`).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The row actions (E07-S08): the context menu's entries, the delete key and the delete
    /// dialog. `None` for a table without them.
    pub actions: Option<ResourceActions>,
}

/// What a resource table tells its owner.
#[derive(Clone, Debug, PartialEq)]
pub enum ResourceTableEvent {
    /// `resource::Open` ran for one of this table's rows: show its detail. The detail drawer
    /// (E07-S05) is opened by [`ResourceViews`](crate::ResourceViews) for the same command; this
    /// event is for whoever else wants to follow it.
    OpenDetail(ResourceRef),
    /// The selection changed.
    SelectionChanged,
    /// The API server sent a `Warning:` header this session has not shown yet (the store
    /// de-duplicates by code and text). The window shows it as a toast.
    ApiWarning(oxikube_ports::ApiWarning),
    /// The "Clear filter" of the empty-because-filtered state ran: the filter bar follows.
    FilterCleared,
}

/// The table of one kind in one cluster. See the [`table`](crate::table) module docs.
pub struct ResourceTable {
    pub(super) cluster: ClusterId,
    pub(super) kind: ResourceKind,
    pub(super) title: SharedString,
    pub(super) deps: ResourceTableDeps,
    pub(super) focus: FocusHandle,
    pub(super) table: TableHandle<RowsDelegate>,
    /// The store the subscription reads, to notice a reconnect (a new store).
    pub(super) store: Option<ResourceStore>,
    pub(super) subscription: Option<oxikube_app::store::Subscription>,
    /// Polls the subscription; replaced with it.
    pub(super) feed_task: Option<Task<()>>,
    /// Forwards the store's `Warning:` headers; replaced with the store.
    pub(super) warning_task: Option<Task<()>>,
    /// The feed kind the provider was chosen for.
    pub(super) feed_kind: Option<FeedKind>,
    /// The versions of the kind the cluster serves (custom kinds, E07-S07), newest first.
    pub(super) served: Vec<ResourceKind>,
    /// Where the columns of the last Table delta came from (`None` for kinds without a Table
    /// feed, and until the feed answers).
    pub(super) columns_source: Option<TableSource>,
    pub(super) writer: Option<PrefsWriter>,
    /// Whether the saved layout has been read (until then the defaults show and nothing is
    /// saved over it).
    pub(super) prefs_loaded: bool,
    pub(super) active: bool,
    /// The cluster tab's workspace, which hosts the delete dialog and the toasts.
    pub(super) workspace: Option<gpui::WeakEntity<Workspace>>,
    /// The shell or attach being set up (the pod read, then the command or the picker); a newer
    /// one replaces, and so cancels, it.
    pub(super) exec_task: Option<Task<()>>,
    /// How many rows are selected (mirrored for the key context, which has no `App`).
    pub(super) selected: usize,
    /// The `/` filter bar in the toolbar (E07-S04).
    pub(super) filter: Entity<FilterBar>,
    /// The filter the subscription runs with: the bar's last good one.
    pub(super) filter_parts: FilterParts,
    /// Whether the filter field has the focus, read from the window on every render (mirrored
    /// for the key context, which has no `App`).
    pub(super) editing: bool,
    /// `/` presses whose `table::FocusFilter` command has not come back yet: the key focuses the
    /// bar at once, so the command's own focus request is only an echo and is skipped.
    pub(super) filter_focus_echoes: u32,
    /// Saves the filter text while `resource_table.persist_filter` is on (made on first save).
    pub(super) filter_writer: Option<FilterWriter>,
    /// Reads the saved filter when the table opens; replaced, never cleared from inside.
    pub(super) filter_task: Option<Task<()>>,
    _session_task: Task<()>,
    pub(super) prefs_task: Option<Task<()>>,
    _tick: Task<()>,
    _subscriptions: Vec<Subscription>,
    /// The feed's redraw: skipped while the table has not rendered the last one (a table in a
    /// background cluster tab is not drawn, E05-P599).
    pub(super) redraw: RenderGate,
    /// How many times the view rendered (coalescing tests).
    #[cfg(test)]
    pub(super) renders: usize,
    /// Added to the clock (age tests).
    #[cfg(test)]
    pub(super) skew: jiff::SignedDuration,
}

impl EventEmitter<ItemEvent> for ResourceTable {}
impl EventEmitter<ResourceTableEvent> for ResourceTable {}

impl ResourceTable {
    /// A table of `kind` in `cluster`. Subscribes to the cluster's store at once (rows arrive
    /// when the feed lists) and reads the saved column layout in the background.
    pub fn new(
        cluster: ClusterId,
        kind: ResourceKind,
        deps: ResourceTableDeps,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title: SharedString = plural_title(&kind).into();
        let provider = initial_provider(&cluster, &kind, &deps);
        let scope = ColumnScope::of(&deps, &cluster, &kind);
        let layout = ColumnLayout::new(
            scope.columns(&*provider, &kind.gvk),
            &ColumnPrefs::default(),
        );
        let delegate = RowsDelegate {
            rows: Vec::new(),
            layout,
            provider,
            selection: Selection::default(),
            state: FeedState::Warming,
            labels: StateLabels::new(
                &kind.plural,
                &resource_label(&kind),
                &scope_label(&WatchScope::Cluster, kind.namespaced),
            ),
            filter: None,
            details_open: false,
            now: Timestamp::now(),
            colors: None,
            cells: CellCache::default(),
            view: cx.entity().downgrade(),
            actions: deps.actions.clone().map(|actions| ActionSource {
                actions,
                cluster: cluster.clone(),
                kind: kind.clone(),
            }),
            #[cfg(test)]
            rendered_cells: 0,
        };
        let options = TableOptions {
            sortable: true,
            resizable_columns: true,
            movable_columns: true,
            loop_selection: false,
            select_rows: false,
        };
        let table = TableHandle::with_options(delegate, options, window, cx);
        let focus = cx.focus_handle();

        let weak = cx.entity().downgrade();
        let events = table.on_event(cx, move |event: &TableEvent, cx| {
            weak.update(cx, |view, cx| view.on_table_event(event, cx))
                .ok();
        });
        // Clicking a header or a row focuses the library's table; the keys live on this view.
        let refocus = cx.on_focus(&table.focus_handle(cx), window, |view, window, cx| {
            window.focus(&view.focus, cx);
        });

        let filter = cx.new(|cx| FilterBar::new(window, cx));
        let filter_events = cx.subscribe_in(&filter, window, Self::on_filter_event);

        let session_task = Self::follow_session(&cluster, &deps, cx);
        let session_echo = Self::follow_session_echo(cx);
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update(cx, |view, cx| {
                    if view.active && view.ages_moved(cx) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        let mut this = Self {
            cluster,
            kind,
            title,
            deps,
            focus,
            table,
            store: None,
            subscription: None,
            feed_task: None,
            warning_task: None,
            feed_kind: None,
            served: Vec::new(),
            columns_source: None,
            writer: None,
            prefs_loaded: false,
            active: false,
            workspace: None,
            exec_task: None,
            selected: 0,
            filter,
            filter_parts: FilterParts::default(),
            editing: false,
            filter_focus_echoes: 0,
            filter_writer: None,
            filter_task: None,
            _session_task: session_task,
            prefs_task: None,
            _tick: tick,
            _subscriptions: vec![events, refocus, filter_events, session_echo],
            redraw: RenderGate::default(),
            #[cfg(test)]
            renders: 0,
            #[cfg(test)]
            skew: jiff::SignedDuration::ZERO,
        };
        this.load_prefs(cx);
        this.load_filter(window, cx);
        this.resubscribe(cx);
        this
    }

    /// The cluster listed.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// The kind listed.
    pub fn kind(&self) -> &ResourceKind {
        &self.kind
    }

    /// The kind's type.
    pub fn gvk(&self) -> &Gvk {
        &self.kind.gvk
    }

    /// Whether a cell on screen would read differently now (an age crossed into its next unit).
    fn ages_moved(&self, cx: &App) -> bool {
        let now = self.now();
        self.table
            .read(cx, |d| d.cells.ages_moved(&*d.provider, now))
    }

    /// "Now" for ages: the clock (tests skew it to move ages without waiting).
    pub(super) fn now(&self) -> Timestamp {
        #[cfg(test)]
        {
            Timestamp::now() + self.skew
        }
        #[cfg(not(test))]
        {
            Timestamp::now()
        }
    }

    /// The filter bar (E07-S04): its text, error and count.
    pub fn filter(&self) -> &Entity<FilterBar> {
        &self.filter
    }

    /// The table handle (rows, layout and selection live in its delegate).
    pub fn table(&self) -> &TableHandle<RowsDelegate> {
        &self.table
    }

    /// Reads the delegate: rows, layout, selection.
    pub fn read_rows<R>(&self, cx: &App, f: impl FnOnce(&RowsDelegate) -> R) -> R {
        self.table.read(cx, f)
    }

    /// The `ResourceRef` of row `ix`.
    pub fn row_ref(&self, ix: usize, cx: &App) -> Option<ResourceRef> {
        Some(self.resource_ref(self.row_key(ix, cx)?))
    }

    /// The `ObjectKey` of row `ix`.
    pub(super) fn row_key(&self, ix: usize, cx: &App) -> Option<ObjectKey> {
        self.table.read(cx, |d| d.row(ix).map(|row| row.key()))
    }

    /// The `ResourceRef` of the object `key` in this table's cluster and kind.
    pub(super) fn resource_ref(&self, key: ObjectKey) -> ResourceRef {
        ResourceRef::new(
            self.cluster.clone(),
            self.kind.gvk.clone(),
            key.namespace,
            key.name,
        )
    }

    /// The `ResourceRef`s of the selected rows, in row order.
    pub fn selected_refs(&self, cx: &App) -> Vec<ResourceRef> {
        self.table.read(cx, |d| {
            d.selection
                .in_row_order(&d.rows)
                .into_iter()
                .map(|key| self.resource_ref(key))
                .collect()
        })
    }
}

/// How RBAC names `kind`'s resource: `pods`, `deployments.apps`.
fn resource_label(kind: &ResourceKind) -> String {
    if kind.gvk.group.is_empty() {
        kind.plural.clone()
    } else {
        format!("{}.{}", kind.plural, kind.gvk.group)
    }
}

/// The tab title of `kind`: its plural, spelled like the kind ("Pods", "NetworkPolicies").
pub fn plural_title(kind: &ResourceKind) -> String {
    let name: &str = &kind.gvk.kind;
    let lower = name.to_lowercase();
    if kind.plural == lower {
        return name.to_owned();
    }
    for suffix in ["s", "es"] {
        if kind.plural == format!("{lower}{suffix}") {
            return format!("{name}{suffix}");
        }
    }
    if let Some(stem) = name.strip_suffix('y')
        && kind.plural == format!("{}ies", stem.to_lowercase())
    {
        return format!("{stem}ies");
    }
    kind.plural.clone()
}

impl Focusable for ResourceTable {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl KeyContextual for ResourceTable {
    const KEY_CONTEXT: &'static str = contexts::RESOURCE_TABLE;

    fn extend_key_context(&self, context: &mut KeyContextBuilder) {
        let selection = match self.selected {
            0 => "none",
            1 => "one",
            _ => "many",
        };
        context.value("selection", selection);
        // Per-kind bindings (`s` is a shell on Pods and Nodes, scale on Deployments) scope on
        // `kind == Pod`; the kind's name is all a keymap section needs.
        context.value("kind", self.kind.gvk.kind.to_string());
        context.value(
            "scope",
            match self.kind.scope() {
                oxikube_domain::ids::Scope::Cluster => "cluster",
                oxikube_domain::ids::Scope::Namespaced => "namespaced",
            },
        );
        // Bare keys (`a`, `s`, `j`, `k`, `/`, enter) are text while the filter field has the focus.
        context.flag_if(self.editing, contexts::EDITING);
    }
}

impl Item for ResourceTable {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new(self.title.clone())
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(item_key(&self.kind.gvk).into())
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.active = active;
        cx.notify();
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        // Releases the store's feed (its grace period starts) and stops polling.
        self.feed_task = None;
        self.warning_task = None;
        self.subscription = None;
    }
}

/// The item key of the table of `gvk` (one per kind in a cluster's workspace).
pub fn item_key(gvk: &Gvk) -> String {
    format!("resources:{gvk}")
}
