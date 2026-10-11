//! Opening a live resource in the manifest editor (E10-S06): "Edit" and "View YAML in Editor"
//! from a table row, the palette, the keys and the detail drawer.
//!
//! | Module | What |
//! |---|---|
//! | `render` | [`render_resource`]: the object as text, kubectl's key order, `managedFields` hidden, Secret values masked; a pure function |
//! | `regions` | where the hidden, masked and dimmed parts are in that text (spanned YAML model) |
//! | `mask` | [`masked_diagnostics`]: a placeholder left in a Secret is an error, so it can never be applied |
//! | `binding` | [`ResourceBinding`]: what an editor opened from an object holds (the object, the toggles, the load state) |
//! | `load` | the fresh `get` on the Kubernetes runtime and the render off the UI thread |
//! | `actions` | the row actions (`resource::Edit`, `resource::ViewYamlInEditor`) |
//!
//! The editor reads the object with a fresh `get` (a table row may be metadata only, ADR 0006),
//! never from the table's cache. Secret values stay in memory: the revealed text is not saved
//! with the layout (the editor item is never serialised) and not logged.

mod actions;
mod binding;
mod load;
mod mask;
mod regions;
mod render;

pub use actions::{EDIT_ORDER, VIEW_ORDER, editor_row_actions};
pub use binding::{OpenMode, ResourceBinding, ResourceLoad, ResourceOpen};
pub use mask::{MASKED_CODE, masked_diagnostics};
pub use render::{
    HiddenKind, HiddenRegion, MASKED, MaskedRegion, RenderError, RenderOptions, RenderedManifest,
    render_resource,
};

#[cfg(test)]
mod tests;
