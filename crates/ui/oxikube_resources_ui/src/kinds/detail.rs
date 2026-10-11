//! [`DetailRenderer`]: how a kind's detail is drawn beyond the generic one.

use std::fmt;
use std::rc::Rc;

use gpui::{AnyView, App, WeakEntity, Window};

use crate::detail::DetailView;

/// Builds a kind's own panel for one detail view. The panel reads the object through the
/// [`DetailView`] handle ([`DetailView::resource`]) and observes it to follow changes; it is
/// built once per detail view, the first time the Overview is drawn.
pub type DetailViewFactory = dyn Fn(WeakEntity<DetailView>, &mut Window, &mut App) -> AnyView;

/// How a kind's detail is drawn.
#[derive(Clone, Default)]
pub enum DetailRenderer {
    /// The generic detail (E07-S05) alone: header, metadata, conditions, status, YAML,
    /// Describe, Events.
    #[default]
    Generic,
    /// The generic detail with the kind's own panel above the Overview (containers of a pod,
    /// the rollout of a deployment, ...).
    Panel(Rc<DetailViewFactory>),
}

impl DetailRenderer {
    /// A renderer that adds the panel `factory` builds.
    pub fn panel(
        factory: impl Fn(WeakEntity<DetailView>, &mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        DetailRenderer::Panel(Rc::new(factory))
    }

    /// Whether this is the generic detail alone.
    pub fn is_generic(&self) -> bool {
        matches!(self, DetailRenderer::Generic)
    }

    /// The kind's panel for `detail`, `None` for the generic detail.
    pub fn build(
        &self,
        detail: WeakEntity<DetailView>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyView> {
        match self {
            DetailRenderer::Generic => None,
            DetailRenderer::Panel(factory) => Some(factory(detail, window, cx)),
        }
    }
}

impl fmt::Debug for DetailRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DetailRenderer::Generic => f.write_str("Generic"),
            DetailRenderer::Panel(_) => f.write_str("Panel(..)"),
        }
    }
}
