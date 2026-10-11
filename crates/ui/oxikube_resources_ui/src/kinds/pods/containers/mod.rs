//! The containers drill-down (E12-S02, k9s' Enter on a pod): a sub-table of the pod's containers
//! at the top of its detail's Overview. Init, sidecar and ephemeral containers are in the same
//! table with a type marker next to the name.
//!
//! [`ContainersPanel`] follows its [`DetailView`]: when the detail's object is a new version (a
//! new JSON document), it rebuilds its [`ContainerRow`]s once, in the observer, and render only
//! lays out the strings it holds. Rows have one height; the table scrolls sideways in a narrow
//! drawer and downwards past `MAX_HEIGHT`.

mod render;
mod rows;

#[cfg(test)]
pub(crate) mod tests;

use gpui::{AnyView, App, AppContext as _, Context, WeakEntity, Window};
use oxikube_domain::json::JsonDoc;

pub use rows::{ContainerRow, rows_of};

use crate::detail::DetailView;

/// The panel's tallest height, unscaled pixels: about ten rows; more scroll.
const MAX_HEIGHT: f32 = 300.;

/// The containers sub-table of one pod detail. See the [module docs](self).
pub struct ContainersPanel {
    detail: WeakEntity<DetailView>,
    /// The document the rows were built from.
    doc: Option<JsonDoc>,
    rows: Vec<ContainerRow>,
    /// How many times the rows were built (tests: once per pod version).
    builds: usize,
}

impl ContainersPanel {
    /// The panel of `detail` (the pod's [`DetailRenderer`](crate::kinds::DetailRenderer)
    /// factory).
    ///
    /// The detail is being drawn when it calls this, so the first rows are built right after
    /// (deferred); until then the panel draws nothing.
    pub fn build(detail: WeakEntity<DetailView>, _: &mut Window, cx: &mut App) -> AnyView {
        let panel = cx.new(|cx| {
            if let Some(view) = detail.upgrade() {
                cx.observe(&view, |this: &mut Self, _, cx| this.refresh(cx))
                    .detach();
            }
            Self {
                detail,
                doc: None,
                rows: Vec::new(),
                builds: 0,
            }
        });
        let first = panel.downgrade();
        cx.defer(move |cx| {
            first.update(cx, |panel, cx| panel.refresh(cx)).ok();
        });
        panel.into()
    }

    /// The rows drawn now.
    pub fn rows(&self) -> &[ContainerRow] {
        &self.rows
    }

    /// How many times the rows were built.
    pub fn builds(&self) -> usize {
        self.builds
    }

    /// Rebuilds the rows when the detail shows another version of the pod.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let built = detail.read(cx).resource().and_then(|pod| {
            let same = self.doc.as_ref().is_some_and(|doc| doc.ptr_eq(pod.doc()));
            (!same).then(|| (pod.doc().clone(), rows_of(pod)))
        });
        let Some((doc, rows)) = built else {
            return;
        };
        self.doc = Some(doc);
        self.rows = rows;
        self.builds += 1;
        cx.notify();
    }
}
