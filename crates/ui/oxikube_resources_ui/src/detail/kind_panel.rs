//! The kind's own panel (E12-S01): what a kind's [`DetailRenderer`] adds above the generic
//! Overview, built once per detail view on its first draw, and [`DetailView::resource`], the
//! object the panel reads.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyView, App, Context, Window};
use oxikube_app::store::StoreObject;
use oxikube_app::{ClusterSessionManager, ColumnsContext};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef, Scope};
use oxikube_ports::DiscoveryPort;
use oxikube_workspace::CommandDispatcher;

use super::state::DetailDeps;
use super::view::DetailView;
use crate::kinds::DetailRenderer;

/// The kind's panel: the renderer until it is built, then the view.
pub(super) enum KindPanel {
    /// Not drawn yet.
    Pending(DetailRenderer),
    /// Built (`None`: the generic detail alone).
    Built(Option<AnyView>),
}

impl KindPanel {
    pub(super) fn new(renderer: DetailRenderer) -> Self {
        if renderer.is_generic() {
            KindPanel::Built(None)
        } else {
            KindPanel::Pending(renderer)
        }
    }

    /// The built panel, if the kind has one.
    pub(super) fn view(&self) -> Option<&AnyView> {
        match self {
            KindPanel::Built(view) => view.as_ref(),
            KindPanel::Pending(_) => None,
        }
    }
}

/// What a kind's panel may do besides reading the object: send a command, ask whether the
/// cluster is read-only, reach discovery. Cheap to clone; taken with [`DetailView::host`].
///
/// A panel never calls a port that mutates: its buttons dispatch commands, which the guard
/// confirms, audits and refuses on a read-only cluster.
#[derive(Clone)]
pub struct PanelHost {
    target: ResourceRef,
    dispatcher: Rc<dyn CommandDispatcher>,
    sessions: ClusterSessionManager,
}

impl PanelHost {
    /// The object the detail shows.
    pub fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Sends `command` through the command bus (the same path as the palette and the keymap).
    pub fn dispatch(&self, command: Command, cx: &mut App) {
        self.dispatcher.dispatch(command, cx);
    }

    /// Whether the object's cluster refuses mutations.
    pub fn read_only(&self) -> bool {
        self.sessions
            .get(&self.target.cluster)
            .is_some_and(|session| session.read_only())
    }

    /// The cluster's discovery, while it is connected.
    pub fn discovery(&self) -> Option<Arc<dyn DiscoveryPort>> {
        self.sessions
            .get(&self.target.cluster)
            .and_then(|session| session.discovery())
    }
}

impl DetailView {
    /// What the kind's panel may do besides reading the object. See [`PanelHost`].
    pub fn host(&self) -> PanelHost {
        PanelHost {
            target: self.target.clone(),
            dispatcher: self.deps.dispatcher.clone(),
            sessions: self.deps.sessions.clone(),
        }
    }

    /// The object as best known: the full read for a feed that carries less, else the feed's
    /// object. `None` until the store answers, and for a Table row without a full read.
    pub fn resource(&self) -> Option<&Resource> {
        self.full
            .resource()
            .or_else(|| match self.object.as_deref() {
                Some(StoreObject::Resource(resource)) => Some(resource),
                _ => None,
            })
    }

    /// A reference to the object `name` of `gvk` in this object's cluster, and in this object's
    /// namespace when `scope` is namespaced: what a panel links to (an HPA's scale target, a
    /// binding's policy).
    pub fn sibling(&self, gvk: Gvk, name: &str, scope: Scope) -> ResourceRef {
        let namespace = match scope {
            Scope::Namespaced => self.target.namespace.clone(),
            Scope::Cluster => None,
        };
        ResourceRef::new(self.target.cluster.clone(), gvk, namespace, name)
    }

    /// Opens the detail of `target`: sends `resource::Open`, as an owner link does.
    pub fn open_resource(&mut self, target: ResourceRef, cx: &mut Context<Self>) {
        self.deps
            .dispatcher
            .dispatch(Command::ResourceOpen { target }, cx);
    }

    /// What the view is built over: the sessions, the stores, the dispatcher. A kind's panel
    /// subscribes to its own feeds and sends its buttons' commands through these.
    pub(crate) fn deps(&self) -> &DetailDeps {
        &self.deps
    }

    /// The command dispatcher the view's own buttons use (a kind panel's buttons send their
    /// commands through it, so they are the bus's, like every other action).
    pub fn dispatcher(&self) -> std::rc::Rc<dyn oxikube_workspace::CommandDispatcher> {
        self.deps.dispatcher.clone()
    }

    /// The cluster, session store and shared catalogue this detail is built over, for a kind's
    /// panel that reads more than the object (the claim panel's Pods).
    pub fn columns_context(&self) -> ColumnsContext {
        let store = self
            .deps
            .sessions
            .get(&self.target.cluster)
            .and_then(|session| self.deps.stores.for_session(&session));
        ColumnsContext {
            cluster: self.target.cluster.clone(),
            sessions: self.deps.sessions.clone(),
            store,
            shared: self.deps.columns.clone(),
        }
    }

    /// The kind's own panel, when its spec has one and it was drawn.
    pub fn kind_panel(&self) -> Option<&AnyView> {
        self.kind_panel.view()
    }

    /// Builds the kind's panel the first time the view is drawn (the factory needs a window).
    pub(super) fn ensure_kind_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let KindPanel::Pending(renderer) = &self.kind_panel {
            let renderer = renderer.clone();
            let view = renderer.build(cx.entity().downgrade(), window, cx);
            self.kind_panel = KindPanel::Built(view);
        }
    }
}
