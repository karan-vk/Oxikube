//! [`ResourceTable`]: the list of one kind in one cluster, as a workspace item: the entity, its
//! construction and the `Item` impl. See the [`table`](crate::table) module docs for the files.
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, SharedString, Subscription, Task, Window,
};
use jiff::Timestamp;
use oxikube_app::store::{FeedKind, FeedState, ObjectKey, ResourceStore, ResourceStores};
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::kinds::ResourceKind;
use oxikube_keymap::{KeyContextBuilder, KeyContextual, contexts};
use oxikube_ports::StatePort;
use oxikube_ui::TableHandle;
use oxikube_ui::table::TableEvent;
use oxikube_ui::table::TableOptions;
use oxikube_workspace::{CommandDispatcher, Item, ItemEvent, TabContent};

use super::columns::{initial_provider, session_capabilities};
use super::delegate::RowsDelegate;
use super::layout::ColumnLayout;
use super::prefs::{ColumnPrefs, PrefsWriter};
use super::selection::Selection;

/// How often ages are redrawn while the table is shown.
const TICK: Duration = Duration::from_secs(1);

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
}

/// What a resource table tells its owner.
#[derive(Clone, Debug, PartialEq)]
pub enum ResourceTableEvent {
    /// `resource::Open` ran for one of this table's rows: show its detail. The detail drawer
    /// (E07-S05) listens for this.
    OpenDetail(ResourceRef),
    /// The selection changed.
    SelectionChanged,
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
    /// The feed kind the provider was chosen for.
    pub(super) feed_kind: Option<FeedKind>,
    pub(super) writer: Option<PrefsWriter>,
    /// Whether the saved layout has been read (until then the defaults show and nothing is
    /// saved over it).
    pub(super) prefs_loaded: bool,
    pub(super) active: bool,
    /// How many rows are selected (mirrored for the key context, which has no `App`).
    pub(super) selected: usize,
    _session_task: Task<()>,
    pub(super) prefs_task: Option<Task<()>>,
    _tick: Task<()>,
    _subscriptions: Vec<Subscription>,
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
        let caps = session_capabilities(&deps, &cluster);
        let layout = ColumnLayout::new(provider.columns(&kind.gvk, caps), &ColumnPrefs::default());
        let delegate = RowsDelegate {
            rows: Vec::new(),
            layout,
            provider,
            selection: Selection::default(),
            state: FeedState::Warming,
            plural: Arc::from(title.to_lowercase()),
            now: Timestamp::now(),
            colors: None,
            view: cx.entity().downgrade(),
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

        let session_task = Self::follow_session(&cluster, &deps, cx);
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update(cx, |view, cx| {
                    if view.active {
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
            feed_kind: None,
            writer: None,
            prefs_loaded: false,
            active: false,
            selected: 0,
            _session_task: session_task,
            prefs_task: None,
            _tick: tick,
            _subscriptions: vec![events, refocus],
        };
        this.load_prefs(cx);
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
    const KEY_CONTEXT: &'static str = contexts::TABLE;

    fn extend_key_context(&self, context: &mut KeyContextBuilder) {
        let selection = match self.selected {
            0 => "none",
            1 => "one",
            _ => "many",
        };
        context.value("selection", selection);
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
        self.subscription = None;
    }
}

/// The item key of the table of `gvk` (one per kind in a cluster's workspace).
pub fn item_key(gvk: &Gvk) -> String {
    format!("resources:{gvk}")
}
