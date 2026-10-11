//! The CustomResourceDefinition kind (E12-S17): its spec, with the CRD list's columns (the app's
//! catalogue: Name, Kind, Group, Versions, Scope, Short Names, Age; Resource, Aliases and
//! Established wide) and the definition panel of its detail.
//!
//! | File | Holds |
//! |---|---|
//! | `mod.rs` | the spec and its registration |
//! | `model` | [`PanelModel`]: the panel's text, built from the domain's [`CrdSummary`](oxikube_domain::view::CrdSummary) once per change |
//! | `panel` | [`CrdPanel`]: names, scope, conversion, the conditions in brief, each version with its flags and printer columns, and the way to the custom resources |
//!
//! The rest of a CRD's detail is shared: the generic Overview (metadata and the full conditions
//! table), YAML and Describe, and the Schema tab (E07-S07). Enter on a CRD row opens the table of
//! its custom resources (`crd::OpenResources`, k9s' Enter); the list itself is the Custom
//! Resources section's "Definitions" entry (`crd::OpenList`), so the kind has no sidebar slot of
//! its own.

mod model;
mod panel;

use gpui::{App, AppContext as _};
use oxikube_app::KindMeta;
use oxikube_domain::command::CommandId;

use super::detail::DetailRenderer;
use super::spec::KindSpec;
use super::specs::KindSpecs;
use crate::crds::crd_gvk;

pub use model::{ColumnLine, FlagTone, PanelModel, VersionModel};
pub use panel::CrdPanel;

/// The CRD kind's spec: no sidebar slot, the action that opens its custom resources, and the
/// definition panel above the generic Overview.
pub fn spec() -> KindSpec {
    let meta = KindMeta::new(
        crd_gvk(),
        "customresourcedefinitions",
        "Definitions",
        oxikube_domain::ids::Scope::Cluster,
    )
    .with_actions([CommandId::CRD_OPEN_RESOURCES]);
    KindSpec::new(meta).with_detail(DetailRenderer::panel(|detail, _, cx| {
        cx.new(|cx| CrdPanel::new(detail, cx)).into()
    }))
}

/// Registers the CRD kind.
pub fn init(cx: &mut App) {
    if let Err(error) = KindSpecs::register(cx, spec()) {
        tracing::error!(%error, "the CRD kind was not registered");
    }
}
