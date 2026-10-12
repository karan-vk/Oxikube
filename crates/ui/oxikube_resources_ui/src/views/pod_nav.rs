//! `pod::JumpToOwner` and `pod::ShowNode` (E12-S03): read the pod, then open the object it points
//! at in the detail drawer.
//!
//! Both read the cluster (one `get` of the pod, plus discovery for the owner's scope) off the UI
//! thread through `spawn_kube`; the drawer opens when the answer arrives. A pod nobody owns, or
//! one the scheduler has not placed, gets a notice instead. Neither changes a cluster.

use gpui::{Context, Window};
use oxikube_app::WorkloadService;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ResourceRef;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::Toast;

use super::controller::ResourceViews;

/// Which neighbour of a pod to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Neighbour {
    /// The controller that owns the pod.
    Owner,
    /// The node the pod runs on.
    Node,
}

impl Neighbour {
    /// What to say when the pod has none.
    fn none(self, pod: &ResourceRef) -> String {
        match self {
            Neighbour::Owner => format!("{} has no owner", pod.name),
            Neighbour::Node => format!("{} is not on a node yet", pod.name),
        }
    }
}

impl ResourceViews {
    /// Opens the owner or the node of `pod` once it has been read. A newer request replaces, and
    /// so cancels, one still in flight.
    pub(super) fn open_neighbour(
        &mut self,
        pod: ResourceRef,
        which: Neighbour,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let service = WorkloadService::new(self.deps.table.sessions.clone());
        let lookup = pod.clone();
        let read = spawn_kube(cx, async move {
            match which {
                Neighbour::Owner => service.owner_of(&lookup).await,
                Neighbour::Node => service.node_of(&lookup).await,
            }
        });
        self.neighbour_task = Some(cx.spawn_in(window, async move |this, cx| {
            let answer = match read.await {
                Ok(answer) => answer,
                Err(error) => Err(error.into()),
            };
            this.update_in(cx, |views, window, cx| {
                views.neighbour_found(&pod, which, answer, window, cx)
            })
            .ok();
        }));
    }

    fn neighbour_found(
        &mut self,
        pod: &ResourceRef,
        which: Neighbour,
        answer: OxiResult<Option<ResourceRef>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match answer {
            Ok(Some(target)) => {
                self.open_detail(&target, window, cx);
            }
            Ok(None) => self.toast(&pod.cluster, Toast::info(which.none(pod)), cx),
            Err(error) => {
                tracing::warn!(%error, %pod, "reading the pod failed");
                self.toast(
                    &pod.cluster,
                    Toast::error(format!("Could not read {}: {}", pod.name, error.message())),
                    cx,
                );
            }
        }
    }
}
