//! ConfigMaps (E12-S11): the key/value detail with a syntax-aware preview.
//!
//! | File | Holds |
//! |---|---|
//! | `panel` | [`ConfigMapPanel`]: the kind's panel above the Overview. The keys are a virtualised list of uniform rows (key, language, size, the first line of the value); a click expands a key into the preview below the list, where the value is coloured by its language (YAML and JSON have a grammar; the other languages show as text) and capped at [`PREVIEW_CAP`](oxikube_domain::view::PREVIEW_CAP). `binaryData` keys show their size only |
//! | `rows` | the stateless drawing of one key |
//!
//! The view-model ([`ConfigMapView`](oxikube_domain::view::ConfigMapView): language detection by key extension then content, the one-line summary, the capped preview) is made off the UI thread
//! from the full object, once per `resourceVersion`; a frame draws only the rows on screen, and
//! the preview's [`CodeView`](oxikube_ui::code_view::CodeView) is created, parsed and coloured
//! when a key is first expanded, never before. Editing a ConfigMap is the editor's `resource::Edit`
//! (`e`); it applies through `resource::Apply` (risk low).

mod panel;
mod rows;

use gpui::{App, AppContext as _};
use oxikube_app::{KindMeta, Section};
use oxikube_domain::ids::{Gvk, Scope};

pub use panel::ConfigMapPanel;

use super::detail::DetailRenderer;
use super::spec::KindSpec;
use super::specs::KindSpecs;

/// Where ConfigMaps sit in the Config section: first.
pub(super) const RANK: u32 = 10;

/// The ConfigMaps kind: the generic table, this panel in the detail.
pub fn spec() -> KindSpec {
    let meta = KindMeta::new(
        Gvk::new("", "v1", "ConfigMap"),
        "configmaps",
        "ConfigMaps",
        Scope::Namespaced,
    )
    .in_section(Section::CONFIG, RANK);
    KindSpec::new(meta).with_detail(DetailRenderer::panel(|detail, _, cx| {
        cx.new(|cx| ConfigMapPanel::new(detail, cx)).into()
    }))
}

/// Registers the ConfigMaps kind.
pub fn init(cx: &mut App) {
    if let Err(error) = KindSpecs::register(cx, spec()) {
        tracing::error!(%error, "the ConfigMap kind was not registered");
    }
}
