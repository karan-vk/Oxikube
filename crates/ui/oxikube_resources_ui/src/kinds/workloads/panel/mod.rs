//! [`WorkloadPanel`]: the kind panel of a Deployment, StatefulSet, ReplicaSet or
//! ReplicationController (E12-S04), drawn above the generic Overview.
//!
//! | File | Holds |
//! |---|---|
//! | `model` | [`PanelModel`]: the replica stats, strategy, selector and pod template as data, built once per object version |
//! | `follow` | the owned subscriptions: started for the object and store, polled, their rows recomputed |
//! | `owned` | the ReplicaSets and Pods the workload owns, found by `ownerReferences`, as capped rows |
//! | `render` | drawing: the header with the Scale button, the stats, the spec lines, the template, the owned sections |
//!
//! # What it shows
//!
//! * **Replicas**: ready / up-to-date / available (a Deployment), ready / updated and the
//!   partition (a StatefulSet), ready / current / desired (a ReplicaSet or ReplicationController).
//! * **Strategy** (rolling parameters, or Recreate / OnDelete), the **selector**, and the spec
//!   lines that matter (min ready seconds, revision history, a StatefulSet's service, paused).
//! * **Pod template**: each container's image, ports and resource requests and limits.
//! * **ReplicaSets** (a Deployment's, the current revision first) and **Pods**, each a link that
//!   opens the object's own detail.
//!
//! Conditions with their reasons are the generic Overview's table just below.
//!
//! # What runs where
//!
//! The model is built when the object's version changes (a JSON read, no I/O). The owned objects
//! come from the namespace's ReplicaSet and Pod feeds in the store, started when the panel first
//! has the object and shared with any table already watching them; a label filter keeps only the
//! workload's pods in the subscription. Deltas are applied off the render path and redraws are
//! coalesced to frame cadence.

mod follow;
mod model;
pub(crate) mod owned;
mod render;

use std::sync::Arc;

use gpui::{AnyView, AppContext as _, Context, Subscription as GpuiSubscription, Task, WeakEntity};
use oxikube_app::store::{ResourceStore, StoreObject, Subscription};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;

pub use model::{ContainerLine, PanelModel, Stat};
pub use owned::{MAX_ROWS, Owned, OwnedRow};

use super::super::DetailRenderer;
use crate::detail::DetailView;

/// Which owned feed a poll task serves.
#[derive(Clone, Copy)]
enum Which {
    ReplicaSets,
    Pods,
}

/// One owned kind's subscription and the objects it delivered.
#[derive(Default)]
struct OwnedFeed {
    subscription: Option<Subscription>,
    items: Vec<Arc<StoreObject>>,
    task: Option<Task<()>>,
}

/// What the owned feeds were started for: a change of any of it starts them again.
#[derive(Clone)]
struct Followed {
    store: ResourceStore,
    uid: Option<Arc<str>>,
    selector: String,
}

/// The workload panel. See the [module docs](self).
pub struct WorkloadPanel {
    detail: WeakEntity<DetailView>,
    pub(super) model: Option<PanelModel>,
    followed: Option<Followed>,
    replicasets: OwnedFeed,
    pods: OwnedFeed,
    /// The rows of the two sections, recomputed when a feed delivers.
    pub(super) replicaset_rows: Owned,
    pub(super) pod_rows: Owned,
    _observe: Option<GpuiSubscription>,
}

/// The renderer the four workload kinds register: this panel above the generic detail.
pub fn renderer() -> DetailRenderer {
    DetailRenderer::panel(|detail, _, cx| -> AnyView {
        cx.new(|cx| WorkloadPanel::new(detail, cx)).into()
    })
}

impl WorkloadPanel {
    /// A panel following `detail`.
    pub fn new(detail: WeakEntity<DetailView>, cx: &mut Context<Self>) -> Self {
        let observe = detail
            .upgrade()
            .map(|view| cx.observe(&view, |this, _, cx| this.refresh(cx)));
        let this = Self {
            detail,
            model: None,
            followed: None,
            replicasets: OwnedFeed::default(),
            pods: OwnedFeed::default(),
            replicaset_rows: Owned::default(),
            pod_rows: Owned::default(),
            _observe: observe,
        };
        // The detail is mid-update while it builds its panel: read it once that update ends.
        let weak = cx.entity().downgrade();
        cx.defer(move |cx| {
            weak.update(cx, |this, cx| this.refresh(cx)).ok();
        });
        this
    }

    /// The model, once the object is known.
    pub fn model(&self) -> Option<&PanelModel> {
        self.model.as_ref()
    }

    /// The ReplicaSet rows (a Deployment's).
    pub fn replicaset_rows(&self) -> &Owned {
        &self.replicaset_rows
    }

    /// The Pod rows.
    pub fn pod_rows(&self) -> &Owned {
        &self.pod_rows
    }

    /// Follows the detail: rebuilds the model when the object's version changed and starts the
    /// owned feeds when they have not been (or the store changed).
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (built, store) = {
            let view = detail.read(cx);
            let version = view
                .resource()
                .and_then(|r| r.meta.resource_version.clone());
            let stale = match (&self.model, &version) {
                (Some(model), Some(version)) => model.version.as_ref() != Some(version),
                (Some(_), None) | (None, _) => true,
            };
            let built = if stale {
                view.resource().and_then(PanelModel::build)
            } else {
                None
            };
            (built, view.connected_store())
        };
        let mut changed = false;
        if let Some(model) = built {
            changed = self.model.as_ref() != Some(&model);
            self.model = Some(model);
        }
        self.follow(store, &detail, cx);
        if changed {
            cx.notify();
        }
    }

    /// Why the Scale button is not usable now, `None` when it is, outer `None` when the panel has
    /// no scale flow to offer (the detail decides: [`DetailView::scale_state`]).
    pub(super) fn scale_state(&self, cx: &gpui::App) -> Option<Option<String>> {
        self.detail.upgrade()?.read(cx).scale_state()
    }

    /// Opens the scale dialog for the workload. Does nothing when the button is disabled.
    pub fn open_scale(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        detail.update(cx, |detail, cx| detail.open_scale(window, cx));
    }

    /// Opens an owned object's detail (`resource::Open`), like an owner link does.
    pub(super) fn open_object(&mut self, target: ResourceRef, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let dispatcher = detail.read(cx).deps().dispatcher.clone();
        dispatcher.dispatch(Command::ResourceOpen { target }, cx);
    }
}
