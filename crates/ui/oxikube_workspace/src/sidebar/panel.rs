//! [`SidebarPanel`]: the cluster's sidebar, a left-dock panel of its tab's workspace.
//!
//! | File | Holds |
//! |---|---|
//! | `panel.rs` | the entity, its inputs and the `Panel` impl (this file) |
//! | `follow.rs` | following the session: reviews, discovery, rebuilding the rows |
//! | `interact.rs` | open and close, highlight, activate |
//! | `view.rs` | drawing the virtualised list |
//! | `counts.rs` | the count badges: a `ResourceStore` read on a timer |
//!
//! # What runs where
//!
//! Nothing here blocks: the rules reviews and discovery run on the Tokio bridge
//! ([`spawn_kube`](oxikube_runtime::spawn_kube)) after the session is connected, so the sidebar
//! never delays `Ready`; the saved state is read and written through the `StatePort` off the UI
//! thread. The panel owns the tasks it starts and replaces them when newer input arrives (a
//! namespace change cancels the review in flight); nothing clears its own task.

use std::collections::BTreeMap;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Entity, EventEmitter, FocusHandle, Focusable, Pixels, SharedString,
    Subscription, Task, UniformListScrollHandle, Window, px,
};
use oxikube_app::{
    ClusterSessionManager, CustomResourceGroup, IntegrationRegistry, IntegrationSection,
    ResourceStores,
};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::StatePort;
use oxikube_ui::IconName;

use super::actions::Toggle;
use super::rows::{AccessState, Row};
use super::section::{SidebarSection, SidebarTarget};
use super::store::SidebarStore;
use super::writer::SidebarWriter;
use crate::panel::{DockPosition, Panel, PanelEvent};

mod counts;
mod follow;
mod interact;
mod view;

/// What a cluster's sidebar needs from the outside.
#[derive(Clone)]
pub struct SidebarDeps {
    /// The sessions: the cluster's ports, selection and capabilities, and their updates.
    pub sessions: ClusterSessionManager,
    /// The integrations whose sections are appended after the core ones.
    pub integrations: IntegrationRegistry,
    /// Where the collapsed state is saved, per cluster.
    pub state: Arc<dyn StatePort>,
    /// The per-cluster resource stores the count badges read (E07-S11). `None`: no badges.
    pub stores: Option<Arc<ResourceStores>>,
}

/// What the sidebar tells whoever hosts it.
#[derive(Clone, Debug, PartialEq)]
pub enum SidebarEvent {
    /// The user went to an entry (pure navigation; the host opens the matching view).
    Navigate(SidebarTarget),
}

/// The default width of the sidebar dock, unscaled pixels.
pub const DEFAULT_WIDTH: f32 = 248.;

/// The sidebar of one cluster. See the module docs.
pub struct SidebarPanel {
    cluster: ClusterId,
    deps: SidebarDeps,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    /// The registry's sections as of the last rebuild.
    sections: Vec<SidebarSection>,
    /// The integrations' sections as of the last rebuild.
    integrations: Vec<IntegrationSection>,
    /// The cluster's custom resources; `None` until discovery answered.
    custom: Option<Vec<CustomResourceGroup>>,
    access: AccessState,
    /// The user's explicit open and closed choices by row id.
    open: BTreeMap<String, bool>,
    rows: Vec<Row>,
    /// The highlighted row's id (keyboard and hover cursor).
    highlighted: Option<SharedString>,
    /// The entry last navigated to.
    selected: Option<SharedString>,
    store: Option<SidebarStore>,
    /// Writes the open and closed choices in order; `None` when there is no store.
    writer: Option<SidebarWriter>,
    /// The count badges: the store read, the lease and the timer.
    counts: counts::CountsState,
    review_task: Option<Task<()>>,
    discovery_task: Option<Task<()>>,
    load_task: Option<Task<()>>,
    /// Follows the session's updates. Lives as long as the panel.
    _watch_session: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for SidebarPanel {}
impl EventEmitter<SidebarEvent> for SidebarPanel {}

impl Focusable for SidebarPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SidebarPanel {
    /// The cluster this sidebar belongs to.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// The rows as drawn.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// What the sidebar knows about the user's access.
    pub fn access(&self) -> &AccessState {
        &self.access
    }

    /// The row with `id`.
    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|row| row.id() == id)
    }

    /// The ids of the visible section headings, in order.
    pub fn visible_sections(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                Row::Section(section) => Some(section.id.to_string()),
                _ => None,
            })
            .collect()
    }

    /// The id of the highlighted row.
    pub fn highlighted(&self) -> Option<&str> {
        self.highlighted.as_deref()
    }

    /// The id of the entry last navigated to.
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }
}

impl Panel for SidebarPanel {
    fn persistent_name() -> &'static str {
        "ClusterSidebar"
    }

    fn panel_key() -> &'static str {
        "cluster_sidebar"
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        DockPosition::Left
    }

    fn default_size(&self, _: &Window, _: &App) -> Pixels {
        px(DEFAULT_WIDTH)
    }

    fn min_size(&self, _: &Window, _: &App) -> Option<Pixels> {
        Some(px(160.))
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::PanelLeft)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<SharedString> {
        Some("Cluster".into())
    }

    fn title(&self, _: &App) -> SharedString {
        "Cluster".into()
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(Toggle)
    }

    fn activation_priority(&self) -> u32 {
        0
    }
}

impl SidebarPanel {
    /// The sidebar of `cluster`, following its session through `deps`.
    pub fn build(cluster: ClusterId, deps: SidebarDeps, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let store = match SidebarStore::new(deps.state.clone(), &cluster) {
                Ok(store) => Some(store),
                Err(error) => {
                    tracing::warn!(%error, %cluster, "the sidebar state is not saved");
                    None
                }
            };
            let writer = store.clone().map(|store| SidebarWriter::spawn(store, cx));
            let mut this = Self {
                cluster,
                deps,
                focus: cx.focus_handle(),
                scroll: UniformListScrollHandle::new(),
                sections: Vec::new(),
                integrations: Vec::new(),
                custom: None,
                access: AccessState::Pending,
                open: BTreeMap::new(),
                rows: Vec::new(),
                highlighted: None,
                selected: None,
                store,
                writer,
                counts: counts::CountsState::default(),
                review_task: None,
                discovery_task: None,
                load_task: None,
                _watch_session: None,
                _subscriptions: Vec::new(),
            };
            this.start(cx);
            this
        })
    }
}
