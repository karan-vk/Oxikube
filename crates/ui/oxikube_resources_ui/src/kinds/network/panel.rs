//! [`NetworkPanel`]: the entity every Network kind's detail panel is. It reads the object of the
//! detail it sits in, keeps the parsed view-model until the object changes, and draws the kind's
//! section (the per-kind files add `impl NetworkPanel` blocks).

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{AnyView, App, Task, WeakEntity, Window};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::network::{
    GATEWAY_API_GROUP, GatewayClassSummary, GatewaySummary, IngressClassSummary, IngressSummary,
    NetworkPolicySummary, ObjectRef, RouteSummary,
};
use oxikube_runtime::spawn_kube;

use super::nav::NavKinds;
use super::widgets::Look;
use crate::detail::{DetailView, PanelHost};

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

    /// Asks discovery which versions the cluster serves, once, for the links to Gateways and
    /// GatewayClasses. Off the UI thread; the task is replaced, never cleared from inside.
    fn fetch_kinds(&mut self, cx: &mut Context<Self>) {
        if self.kinds.is_known() || self.kinds_task.is_some() {
            return;
        }
        let Some(discovery) = self.host.as_ref().and_then(PanelHost::discovery) else {
            return;
        };
        let discovered = spawn_kube(cx, async move { discovery.discover().await });
        self.kinds_task = Some(cx.spawn(async move |this, cx| {
            if let Ok(Ok(kinds)) = discovered.await {
                this.update(cx, |this, cx| {
                    this.kinds = NavKinds::new(kinds);
                    cx.notify();
                })
                .ok();
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

    /// Opens the detail of `target` (`resource::Open`), when its kind can be placed.
    pub(super) fn open(&self, target: &ObjectRef, cx: &mut App) {
        if let Some(target) = self.target_of(target) {
            self.dispatch(Command::ResourceOpen { target }, cx);
        }
    }

    /// The reference to open for `target`, `None` while it cannot be placed.
    pub(super) fn target_of(&self, target: &ObjectRef) -> Option<ResourceRef> {
        self.kinds.target(
            self.object()?,
            &target.group,
            &target.kind,
            target.namespace.as_deref(),
            &target.name,
        )
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
