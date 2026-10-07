//! What a [`DetailView`](super::DetailView) is built over and what it keeps: the deps, the
//! mounting mode, how the object stands, the full-object read and the Events tab's feed.

use std::rc::Rc;
use std::sync::Arc;

use gpui::Task;
use oxikube_app::store::{StoreObject, Subscription};
use oxikube_app::{ClusterSessionManager, CoreColumns, ResourceStores};
use oxikube_domain::Resource;
use oxikube_workspace::CommandDispatcher;

use super::events::EventRow;
use crate::exec::ExecFlow;
use crate::table::ResourceTableDeps;

/// What a [`DetailView`](crate::detail::DetailView) is built over. Cheap to clone.
#[derive(Clone)]
pub struct DetailDeps {
    /// The sessions: the cluster's connection, ports and namespace selection.
    pub sessions: ClusterSessionManager,
    /// The per-session resource stores the view subscribes to.
    pub stores: Arc<ResourceStores>,
    /// The column catalogue: the status chip is the kind's own status cell.
    pub columns: Arc<CoreColumns>,
    /// Where the view's commands go (`resource::Open` for an owner, `resource::PinDetail`,
    /// `resource::CopyLabel`).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// Opens a shell or an attach from a pod's header (E09-S08): the pod's cluster tab's
    /// workspace asks which container. `None`: the header has no such buttons.
    pub exec: Option<ExecFlow>,
}

impl From<&ResourceTableDeps> for DetailDeps {
    fn from(deps: &ResourceTableDeps) -> Self {
        Self {
            sessions: deps.sessions.clone(),
            stores: deps.stores.clone(),
            columns: deps.columns.clone(),
            dispatcher: deps.dispatcher.clone(),
            exec: None,
        }
    }
}

/// Where the view is mounted. It is one view with two mounting modes: the mode only changes
/// which buttons the header offers, never the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mount {
    /// In the drawer: the right-hand dock of the cluster tab.
    Drawer,
    /// As a tab of a pane.
    Tab,
}

/// What a detail view tells whoever hosts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailEvent {
    /// The user closed the drawer (the close button).
    Close,
}

/// How the object stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailState {
    /// Waiting for the store's first answer (or for the connection).
    Loading,
    /// The object exists and is followed live.
    Live,
    /// The object was there and the store removed it: the last known state stays on screen,
    /// marked as deleted.
    Deleted,
    /// The feed is ready and has no such object.
    NotFound,
    /// The feed cannot serve the kind (forbidden, failed): why, as the store reported it.
    Unavailable(String),
}

/// The complete object, for kinds whose feed carries less than that (metadata-only feeds, Table
/// feeds).
#[derive(Clone, Default)]
pub(super) enum FullState {
    /// Not needed (the feed carries the whole object) or not asked for yet.
    #[default]
    Idle,
    /// The read is in flight.
    Loading,
    /// Read; secret values are already removed.
    Loaded(Arc<Resource>),
    /// The read failed; the model keeps what the feed has.
    Failed(String),
}

impl FullState {
    pub(super) fn resource(&self) -> Option<&Resource> {
        match self {
            FullState::Loaded(resource) => Some(resource),
            _ => None,
        }
    }
}

/// The Events tab's feed and what it found.
#[derive(Default)]
pub(super) struct EventsTab {
    pub(super) started: bool,
    /// Every event of the namespace as the store has them (the filter reads these).
    pub(super) feed: Vec<Arc<StoreObject>>,
    pub(super) rows: Vec<EventRow>,
    pub(super) ready: bool,
    pub(super) error: Option<String>,
    pub(super) subscription: Option<Subscription>,
    pub(super) task: Option<Task<()>>,
}
