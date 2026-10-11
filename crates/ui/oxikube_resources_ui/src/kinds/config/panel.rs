//! [`ConfigPanel`]: the entity a Config kind's [`DetailRenderer`](crate::kinds::DetailRenderer)
//! builds. It follows the detail view, rebuilds its [`Panel`] only when the object's resource
//! version changes, and draws it (`draw`).

use std::sync::Arc;

use gpui::{
    AnyView, App, AppContext as _, Context, IntoElement, Render, Subscription, WeakEntity, Window,
    div,
};
use oxikube_domain::Resource;
use oxikube_ui::ActiveTokens as _;

use super::draw;
use super::model::{LinkTo, Panel};
use crate::detail::DetailView;
use crate::kinds::DetailRenderer;
use crate::table::ToneColors;

/// Builds a kind's [`Panel`] from its object; `None` for an object it cannot read.
pub(super) type Build = fn(&Resource) -> Option<Panel>;

/// The panel in the detail drawer of a Config kind.
pub struct ConfigPanel {
    detail: WeakEntity<DetailView>,
    build: Build,
    built: Option<Built>,
    _follow: Option<Subscription>,
}

/// A built panel and the object (name, namespace and resource version) it was built from.
struct Built {
    name: Arc<str>,
    namespace: Option<Arc<str>>,
    version: Option<Arc<str>>,
    panel: Arc<Panel>,
}

impl ConfigPanel {
    fn new(detail: WeakEntity<DetailView>, build: Build, cx: &mut Context<Self>) -> Self {
        // The detail view notifies on every delta of the object; the panel redraws with it.
        let follow = detail
            .upgrade()
            .map(|view| cx.observe(&view, |_, _, cx| cx.notify()));
        Self {
            detail,
            build,
            built: None,
            _follow: follow,
        }
    }

    /// The renderer that gives a kind this panel, built with `build`.
    pub(super) fn renderer(build: Build) -> DetailRenderer {
        DetailRenderer::panel(move |detail, _, cx| -> AnyView {
            cx.new(|cx| Self::new(detail, build, cx)).into()
        })
    }

    /// The panel for the object as the detail view has it now; rebuilt only when the object's
    /// name or resource version differs from the last build.
    pub(crate) fn model(&mut self, cx: &App) -> Option<Arc<Panel>> {
        let detail = self.detail.upgrade()?;
        let resource = detail.read(cx).resource()?;
        let version = resource.meta.resource_version.clone();
        if let Some(built) = &self.built
            && version.is_some()
            && built.version == version
            && built.name == resource.meta.name
            && built.namespace == resource.meta.namespace
        {
            return Some(built.panel.clone());
        }
        let panel = Arc::new((self.build)(resource)?);
        self.built = Some(Built {
            name: resource.meta.name.clone(),
            namespace: resource.meta.namespace.clone(),
            version,
            panel: panel.clone(),
        });
        Some(panel)
    }

    /// Opens the object a link row points to, in the detail view's cluster and namespace.
    pub(super) fn open(&mut self, to: &LinkTo, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        detail.update(cx, |view, cx| {
            let target = view.sibling(to.gvk.clone(), &to.name, to.scope);
            view.open_resource(target, cx);
        });
    }
}

impl Render for ConfigPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(panel) = self.model(cx) else {
            return div().into_any_element();
        };
        let (tokens, tones) = (cx.tokens(), ToneColors::current(cx));
        draw::panel(&panel, &tokens, &tones, cx)
    }
}
