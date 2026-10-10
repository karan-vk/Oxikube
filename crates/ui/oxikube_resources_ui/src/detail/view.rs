//! [`DetailView`]: the detail of one object, as an entity that is both the drawer's content and a
//! workspace tab. See the [`detail`](crate::detail) module docs for the files.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, ListAlignment, ListState, SharedString,
    Task, WeakEntity, Window, px,
};
use jiff::Timestamp;
use oxikube_app::ColumnProvider;
use oxikube_app::store::{ResourceStore, StoreObject, Subscription};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef, Scope};
use oxikube_ui::IconName;
use oxikube_workspace::{Item, ItemEvent, TabContent};

use super::ages::TICK;
use super::describe::DescribeTab;
use super::events::EventRow;
use super::model::{DetailModel, OwnerLink, Row};
use super::schema_tab::SchemaPane;
use super::state::{DetailDeps, DetailEvent, DetailState, EventsTab, FullState, Mount};
use super::tabs::DetailTab;
use super::yaml::YamlTab;
use crate::table::ResourceTable;

/// The detail of one object. See the [module docs](crate::detail).
pub struct DetailView {
    pub(super) target: ResourceRef,
    pub(super) deps: DetailDeps,
    pub(super) mount: Mount,
    pub(super) focus: FocusHandle,
    /// The table the drawer was opened from, which `j` / `k` step through.
    pub(super) origin: Option<WeakEntity<ResourceTable>>,
    pub(super) tab: DetailTab,
    /// Whether the view is on screen (ages are checked only then).
    pub(super) shown: bool,
    /// The time the last frame read its ages at: the age tick redraws when one reads differently
    /// now (E07-F566).
    pub(super) drawn_at: Timestamp,
    /// A fixed "now" for ages (screenshots), instead of the clock.
    pub(super) now_override: Option<Timestamp>,
    pub(super) state: DetailState,
    /// The object as the store last had it.
    pub(super) object: Option<Arc<StoreObject>>,
    /// The subscription's own list (at most the one object).
    pub(super) feed: Vec<Arc<StoreObject>>,
    pub(super) provider: Arc<dyn ColumnProvider>,
    pub(super) full: FullState,
    /// The next delivery of the object reads it in full again even at the same version: set on
    /// resubscribing after a failed (or never started) read, so a reconnect gives it another try.
    pub(super) refetch_full: bool,
    pub(super) model: Option<DetailModel>,
    /// The Overview's rows (from the model) and their list state, which keeps the scroll.
    pub(super) body: Vec<Row>,
    pub(super) overview: ListState,
    /// Expanded values: (is annotation, key).
    pub(super) expanded: HashSet<(bool, Arc<str>)>,
    /// What discovery said about each owner's scope: a link opens only once it is known.
    pub(super) owner_scopes: HashMap<Gvk, Option<Scope>>,
    pub(super) events: EventsTab,
    pub(super) yaml: YamlTab,
    pub(super) describe: DescribeTab,
    pub(super) events_list: ListState,
    /// The Schema tab of a CRD (E07-S07).
    pub(super) schema: SchemaPane,
    pub(super) store: Option<ResourceStore>,
    /// The store the Events feed is subscribed on.
    pub(super) events_store: Option<ResourceStore>,
    pub(super) subscription: Option<Subscription>,
    pub(super) feed_task: Option<Task<()>>,
    pub(super) full_task: Option<Task<()>>,
    pub(super) owners_task: Option<Task<()>>,
    /// The shell or attach being set up from the header; a newer one replaces, and so cancels, it.
    pub(super) exec_task: Option<Task<()>>,
    /// How many Overview rows were built (virtualisation tests).
    #[cfg(test)]
    pub(super) rendered_rows: usize,
    /// How many times the view was drawn (age tick tests).
    #[cfg(test)]
    pub(super) renders: usize,
    /// Added to the clock (age tick tests).
    #[cfg(test)]
    pub(super) skew: jiff::SignedDuration,
    _session_task: Task<()>,
    _tick: Task<()>,
}

impl EventEmitter<ItemEvent> for DetailView {}
impl EventEmitter<DetailEvent> for DetailView {}

impl DetailView {
    /// The detail of `target`, mounted as `mount`. Subscribes to the object in the cluster's
    /// store at once; the first frame shows a skeleton until it answers.
    pub fn new(
        target: ResourceRef,
        deps: DetailDeps,
        mount: Mount,
        cx: &mut Context<Self>,
    ) -> Self {
        let session_task = Self::follow_session(&target, &deps, cx);
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update(cx, |view, cx| view.tick_ages(cx));
                if alive.is_err() {
                    break;
                }
            }
        });
        let provider: Arc<dyn ColumnProvider> = deps.columns.clone();
        let mut this = Self {
            target,
            deps,
            mount,
            focus: cx.focus_handle(),
            origin: None,
            tab: DetailTab::Overview,
            shown: true,
            drawn_at: Timestamp::now(),
            now_override: None,
            state: DetailState::Loading,
            object: None,
            feed: Vec::new(),
            provider,
            full: FullState::Idle,
            refetch_full: false,
            model: None,
            body: Vec::new(),
            overview: ListState::new(0, ListAlignment::Top, px(240.)),
            expanded: HashSet::new(),
            owner_scopes: HashMap::new(),
            events: EventsTab::default(),
            yaml: YamlTab::default(),
            describe: DescribeTab::default(),
            events_list: ListState::new(0, ListAlignment::Top, px(240.)),
            schema: SchemaPane::default(),
            store: None,
            events_store: None,
            subscription: None,
            feed_task: None,
            full_task: None,
            owners_task: None,
            exec_task: None,
            #[cfg(test)]
            rendered_rows: 0,
            #[cfg(test)]
            renders: 0,
            #[cfg(test)]
            skew: jiff::SignedDuration::ZERO,
            _session_task: session_task,
            _tick: tick,
        };
        this.resubscribe(cx);
        // The command palette asks the focused detail what it acts on (E11-S03).
        oxikube_workspace::command_surface::register(&cx.entity(), &this.focus, cx);
        this
    }

    /// Sets the table `j` / `k` step through: the one the detail was opened from.
    pub fn set_origin(&mut self, table: Option<WeakEntity<ResourceTable>>) {
        self.origin = table;
    }

    /// The object shown.
    pub fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Where the view is mounted.
    pub fn mount(&self) -> Mount {
        self.mount
    }

    /// Mounts the view as `mount` (the drawer promoting it to a tab, or the reverse). Nothing
    /// else changes: the active tab, the scroll and the expanded values stay.
    pub fn set_mount(&mut self, mount: Mount, cx: &mut Context<Self>) {
        if self.mount != mount {
            self.mount = mount;
            cx.notify();
        }
    }

    /// Fixes the "now" the ages are read against (a screenshot must not change with the clock).
    pub fn pin_now(&mut self, now: Timestamp, cx: &mut Context<Self>) {
        self.now_override = Some(now);
        cx.notify();
    }

    /// The time ages are read against.
    pub(super) fn now(&self) -> Timestamp {
        self.now_override.unwrap_or_else(|| {
            #[cfg(test)]
            {
                Timestamp::now() + self.skew
            }
            #[cfg(not(test))]
            {
                Timestamp::now()
            }
        })
    }

    /// Tells the view whether it is on screen (the drawer's active state).
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown != shown {
            self.shown = shown;
            cx.notify();
        }
    }

    /// How the object stands.
    pub fn state(&self) -> &DetailState {
        &self.state
    }

    /// The active tab.
    pub fn tab(&self) -> DetailTab {
        self.tab
    }

    /// Switches to `tab`. The Events feed starts the first time that tab is shown.
    pub fn set_tab(&mut self, tab: DetailTab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        let left = std::mem::replace(&mut self.tab, tab);
        if left == DetailTab::Schema {
            self.schema_tab_closed();
        }
        match tab {
            DetailTab::Events => self.start_events(cx),
            DetailTab::Yaml => self.refresh_yaml(cx),
            DetailTab::Describe => self.start_describe(cx),
            DetailTab::Schema => self.schema_tab_opened(),
            DetailTab::Overview => {}
        }
        cx.notify();
    }

    /// What the Overview shows, once the object is known.
    pub fn model(&self) -> Option<&DetailModel> {
        self.model.as_ref()
    }

    /// The Overview's rows.
    pub fn rows(&self) -> &[Row] {
        &self.body
    }

    /// The events listed in the Events tab.
    pub fn event_rows(&self) -> &[EventRow] {
        &self.events.rows
    }

    /// The Overview's list state (its scroll position survives tab switches and promotion).
    pub fn overview_list(&self) -> &ListState {
        &self.overview
    }

    /// The `ResourceRef` a link to `owner` opens, once discovery has said whether the owner's
    /// kind is namespaced.
    pub fn owner_target(&self, owner: &OwnerLink) -> Option<ResourceRef> {
        let scope = (*self.owner_scopes.get(&owner.gvk)?)?;
        let namespace = match scope {
            Scope::Namespaced => self.target.namespace.clone(),
            Scope::Cluster => None,
        };
        Some(ResourceRef::new(
            self.target.cluster.clone(),
            owner.gvk.clone(),
            namespace,
            owner.name.clone(),
        ))
    }

    /// Opens the owner `index` of the model: sends `resource::Open` for it.
    pub fn open_owner(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(owner) = self.model.as_ref().and_then(|m| m.owners.get(index)) else {
            return;
        };
        if let Some(target) = self.owner_target(owner) {
            self.deps
                .dispatcher
                .dispatch(Command::ResourceOpen { target }, cx);
        }
    }

    /// Copies the label (or annotation) `key` as `key=value`: sends `resource::CopyLabel`.
    pub fn copy_meta(&mut self, key: &str, annotation: bool, cx: &mut Context<Self>) {
        let command = Command::ResourceCopyLabel {
            target: self.target.clone(),
            key: key.to_owned(),
            annotation,
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Pins the drawer as a tab: sends `resource::PinDetail`.
    pub fn request_pin(&mut self, cx: &mut Context<Self>) {
        let command = Command::ResourcePinDetail {
            target: self.target.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// The `key=value` text of a label or annotation, for the copy command.
    pub fn copy_text(&self, key: &str, annotation: bool) -> Option<String> {
        let entry = self.model.as_ref()?.meta_entry(key, annotation)?;
        entry.copyable.then(|| entry.copy_text())
    }

    /// The drawer's close button.
    pub fn request_close(&mut self, cx: &mut Context<Self>) {
        cx.emit(DetailEvent::Close);
    }

    /// The complete object the view holds, as JSON text (tests check what it keeps of a Secret).
    #[cfg(test)]
    pub(super) fn full_state_text(&self) -> String {
        self.full
            .resource()
            .map(|resource| resource.to_value().to_string())
            .unwrap_or_default()
    }

    /// Releases the feeds (the view is going away).
    pub(super) fn release(&mut self) {
        self.feed_task = None;
        self.subscription = None;
        self.full_task = None;
        self.owners_task = None;
        self.describe.task = None;
        self.yaml.task = None;
        self.events.task = None;
        self.events.subscription = None;
    }
}

impl Focusable for DetailView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Item for DetailView {
    fn tab_content(&self, _: &App) -> TabContent {
        let kind = self.target.gvk.kind.to_lowercase();
        TabContent::new(format!("{kind}/{}", self.target.name)).icon(IconName::FileText)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(item_key(&self.target).into())
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.set_shown(active, cx);
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.release();
    }
}

/// The item key of the detail of `target` (one per object in a cluster's workspace).
pub fn item_key(target: &ResourceRef) -> String {
    format!("detail:{target}")
}

/// The command palette (E11-S03) reads a focused detail as a `Detail` view acting on its object.
impl oxikube_workspace::command_surface::CommandSurface for DetailView {
    fn view_context(&self) -> oxikube_domain::command::ViewContext {
        oxikube_domain::command::ViewContext::Detail
    }

    fn command_target(&self, _: &App) -> oxikube_app::CommandTarget {
        oxikube_app::CommandTarget::none()
            .in_cluster(self.target.cluster.clone())
            .of_kind(self.target.gvk.clone())
            .selecting(vec![self.target.clone()])
    }
}
