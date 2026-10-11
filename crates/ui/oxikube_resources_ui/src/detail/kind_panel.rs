//! The kind's own panel (E12-S01): what a kind's [`DetailRenderer`] adds above the generic
//! Overview, built once per detail view on its first draw, and [`DetailView::resource`], the
//! object the panel reads.

use gpui::{AnyView, Context, Window};
use oxikube_app::store::{ResourceStore, StoreObject};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef, Scope};

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

impl DetailView {
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

    /// The store of the object's cluster while the session is connected: where a kind's panel
    /// subscribes to the objects it lists beside the one shown.
    pub(crate) fn followed_store(&self) -> Option<ResourceStore> {
        self.connected_store()
    }

    /// The time ages are read against.
    pub(crate) fn clock(&self) -> jiff::Timestamp {
        self.now()
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
