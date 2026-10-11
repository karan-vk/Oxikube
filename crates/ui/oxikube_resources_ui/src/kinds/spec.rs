//! [`KindSpec`]: one kind's registration as the UI sees it.

use oxikube_app::KindMeta;

use super::detail::DetailRenderer;

/// Everything about one kind: `{ gvk, columns, detail, actions, sidebar }` plus its templates.
///
/// The data half is the app's [`KindMeta`] (no gpui, testable with fakes, reusable by the MCP
/// server); this adds the [`DetailRenderer`], a gpui view factory. Build one with
/// [`KindSpec::new`], register it with [`KindSpecs::register`](super::KindSpecs::register) from
/// the kind module's `init(cx)`.
#[derive(Clone, Debug)]
pub struct KindSpec {
    /// The kind's data: gvk, plural, title, scope, columns, actions, sidebar slot, templates.
    pub meta: KindMeta,
    /// How its detail is drawn.
    pub detail: DetailRenderer,
}

impl KindSpec {
    /// The spec of `meta` with the generic detail.
    pub fn new(meta: KindMeta) -> Self {
        Self {
            meta,
            detail: DetailRenderer::Generic,
        }
    }

    /// Sets how the detail is drawn.
    #[must_use]
    pub fn with_detail(mut self, detail: DetailRenderer) -> Self {
        self.detail = detail;
        self
    }
}
