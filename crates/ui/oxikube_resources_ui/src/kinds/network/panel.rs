//! [`NetworkPanel`]: the entity every Network kind's detail panel is. It reads the object of the
//! detail it sits in, keeps the parsed view-model until the object changes, and draws the kind's
//! section (the per-kind files add `impl NetworkPanel` blocks).

use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{AnyView, App, Div, ElementId, SharedString, Stateful, Task, WeakEntity, Window};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::network::{
    GATEWAY_API_GROUP, GatewayClassSummary, GatewaySummary, IngressClassSummary, IngressSummary,
    NetworkPolicySummary, ObjectRef, RouteSummary,
};
use oxikube_runtime::spawn_kube;

use super::nav::{KindsCache, NavKinds};
use super::widgets::{Look, link};
use crate::detail::{DetailView, PanelHost};

/// How long after a failed discovery the panel asks again.
const RETRY_AFTER: Duration = Duration::from_secs(5);

/// The view-model of the object being shown.
pub(super) enum Model {
    /// Nothing read yet, or a kind this panel does not know.
    None,
    Ingress(IngressSummary),
    IngressClass(IngressClassSummary),
    Policy(NetworkPolicySummary),
    GatewayClass(GatewayClassSummary),
    Gateway(GatewaySummary),
    Route(RouteSummary),
}

impl Model {
    /// Reads `res` as whichever Network kind it is.
    fn of(res: &Resource) -> Self {
        let group: &str = &res.kind.group;
        match (group, &*res.kind.kind) {
            ("networking.k8s.io", "Ingress") => {
                IngressSummary::from_resource(res).map_or(Model::None, Model::Ingress)
            }
            ("networking.k8s.io", "IngressClass") => {
                IngressClassSummary::from_resource(res).map_or(Model::None, Model::IngressClass)
            }
            ("networking.k8s.io", "NetworkPolicy") => {
                NetworkPolicySummary::from_resource(res).map_or(Model::None, Model::Policy)
            }
            (GATEWAY_API_GROUP, "GatewayClass") => {
                GatewayClassSummary::from_resource(res).map_or(Model::None, Model::GatewayClass)
            }
            (GATEWAY_API_GROUP, "Gateway") => {
                GatewaySummary::from_resource(res).map_or(Model::None, Model::Gateway)
            }
            (GATEWAY_API_GROUP, "HTTPRoute" | "GRPCRoute" | "TLSRoute") => {
                RouteSummary::from_resource(res).map_or(Model::None, Model::Route)
            }
            _ => Model::None,
        }
    }

    /// Whether the panel follows references to kinds only discovery can place.
    fn follows_gateway_api(&self) -> bool {
        matches!(self, Model::Gateway(_) | Model::Route(_))
    }
}

/// Which version of which object the model was read from.
#[derive(PartialEq, Eq)]
struct Seen {
    uid: Option<Arc<str>>,
    resource_version: Option<Arc<str>>,
}

/// See the [module docs](self).
pub(super) struct NetworkPanel {
    pub(super) detail: WeakEntity<DetailView>,
    /// Taken from the detail on the first draw: the detail is being updated while it builds the
    /// panel, so the factory cannot read it.
    host: Option<PanelHost>,
    pub(super) model: Model,
    seen: Option<Seen>,
    pub(super) kinds: NavKinds,
    kinds_task: Option<Task<()>>,
    /// The last discovery failed and the pause is over: ask again on the next draw.
    kinds_retry: bool,
}

impl NetworkPanel {
    /// The factory a spec's [`DetailRenderer`](crate::kinds::DetailRenderer) calls.
    pub(super) fn build(detail: WeakEntity<DetailView>, _: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|cx| {
            if let Some(view) = detail.upgrade() {
                // The detail redraws when the object changes (the feed, the full read): so does
                // the panel.
                cx.observe(&view, |_, _, cx| cx.notify()).detach();
            }
            Self {
                detail,
                host: None,
                model: Model::None,
                seen: None,
                kinds: NavKinds::default(),
                kinds_task: None,
                kinds_retry: false,
            }
        })
        .into()
    }

    /// Reads the detail's object again when it is another version than the one held.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        if self.host.is_none() {
            self.host = Some(detail.read(cx).host());
        }
        let parsed = detail.read(cx).resource().map(|res| {
            let seen = Seen {
                uid: res.meta.uid.clone(),
                resource_version: res.meta.resource_version.clone(),
            };
            (seen, res)
        });
        let Some((seen, res)) = parsed else {
            return;
        };
        if self.seen.as_ref() == Some(&seen) && !matches!(self.model, Model::None) {
            return;
        }
        let model = Model::of(res);
        self.seen = Some(seen);
        self.model = model;
    }

    /// Asks discovery which versions the cluster serves, for the links to Gateways and
    /// GatewayClasses. A recent answer for the connection is shared by every panel (no discovery
    /// per opened detail); off the UI thread otherwise. A failed answer is asked again
    /// [`RETRY_AFTER`] later. The task is replaced, never cleared from inside.
    fn fetch_kinds(&mut self, cx: &mut Context<Self>) {
        if self.kinds.is_known() || (self.kinds_task.is_some() && !self.kinds_retry) {
            return;
        }
        let Some(discovery) = self.host.as_ref().and_then(PanelHost::discovery) else {
            return;
        };
        let Some(cluster) = self.object().map(|o| o.cluster.clone()) else {
            return;
        };
        if let Some(known) = cx.default_global::<KindsCache>().get(&cluster, &discovery) {
            self.kinds = known;
            return;
        }
        self.kinds_retry = false;
        let port = discovery.clone();
        let discovered = spawn_kube(cx, async move { discovery.discover().await });
        self.kinds_task = Some(cx.spawn(async move |this, cx| {
            match discovered.await {
                Ok(Ok(kinds)) => {
                    this.update(cx, |this, cx| {
                        let kinds = NavKinds::new(kinds);
                        cx.default_global::<KindsCache>()
                            .put(cluster, &port, kinds.clone());
                        this.kinds = kinds;
                        cx.notify();
                    })
                    .ok();
                }
                Ok(Err(_)) | Err(_) => {
                    // Unreachable or throttled, or the task was cancelled: the next draw after
                    // the pause asks again.
                    cx.background_executor().timer(RETRY_AFTER).await;
                    this.update(cx, |this, cx| {
                        this.kinds_retry = true;
                        cx.notify();
                    })
                    .ok();
                }
            }
        }));
    }

    /// Sends `command` through the bus; nothing before the first draw.
    pub(super) fn dispatch(&self, command: Command, cx: &mut App) {
        if let Some(host) = &self.host {
            host.dispatch(command, cx);
        }
    }

    /// The object the detail shows.
    pub(super) fn object(&self) -> Option<&ResourceRef> {
        self.host.as_ref().map(PanelHost::target)
    }

    /// Whether the cluster refuses mutations.
    pub(super) fn read_only(&self) -> bool {
        self.host.as_ref().is_some_and(PanelHost::read_only)
    }

    /// A link to `target` that opens its detail (`resource::Open`). Plain text while the kind
    /// cannot be placed (discovery has not answered, or the cluster does not serve it).
    pub(super) fn object_link(
        &self,
        look: &Look,
        id: impl Into<ElementId>,
        selector: String,
        text: impl Into<SharedString>,
        target: &ObjectRef,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = self.object().and_then(|from| {
            self.kinds.target(
                from,
                &target.group,
                &target.kind,
                target.namespace.as_deref(),
                &target.name,
            )
        });
        link(look, id, selector, text, open.is_some()).when_some(open, |el, target| {
            el.on_click(cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    Command::ResourceOpen {
                        target: target.clone(),
                    },
                    cx,
                );
            }))
        })
    }
}

impl Render for NetworkPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh(cx);
        // Also here, not only when the object changes: the cluster may not have been connected
        // (no discovery) when the object arrived.
        if self.model.follows_gateway_api() {
            self.fetch_kinds(cx);
        }
        let look = Look::of(cx);
        match &self.model {
            Model::None => gpui::div().into_any_element(),
            Model::Ingress(summary) => self.ingress(&look, summary, cx),
            Model::IngressClass(summary) => self.ingress_class(&look, summary, cx),
            Model::Policy(summary) => self.policy(&look, summary),
            Model::GatewayClass(summary) => self.gateway_class(&look, summary),
            Model::Gateway(summary) => self.gateway(&look, summary, cx),
            Model::Route(summary) => self.route(&look, summary, cx),
        }
    }
}
