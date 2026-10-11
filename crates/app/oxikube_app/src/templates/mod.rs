//! Templates for new resources (E10-S09): [`TemplateService`] lists the manifests the "Create
//! resource" picker offers and fills the namespace in.
//!
//! ```text
//! embedded templates (passed in by the editor, one minimal manifest per core kind)
//!   + ~/.config/oxikube/templates/*.yaml (read through FsPort, scanned each time the picker opens)
//!   ──> TemplateCatalog (a user template named like an embedded one replaces it)
//!   ──> Template::prefill(NamespaceSelection) ──> the text a new editor tab opens with
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `template` | [`Template`], [`TemplateOrigin`]: a name, a summary, the kind it makes and the text; the header comments (`# name:`, `# description:`) of a user file |
//! | `prefill` | [`Prefill`]: the `namespace: default` placeholder replaced by the selection, with the note when the selection did not name one namespace |
//! | `service` | [`TemplateService`], [`TemplateCatalog`], [`SkippedTemplate`]: the scan of the user directory through `FsPort` |
//!
//! The embedded manifests live with the editor (`oxikube_editor::templates`) and are passed in:
//! this module holds no UI type and no file of its own. Nothing here writes a cluster or a
//! file: the objects are made by `resource::Apply`, one per document, through the
//! `MutationGuard`. A template's text may hold a Secret's `stringData`, so it is never logged:
//! a rejected user file is named by its file name and a generic reason only.

mod prefill;
mod service;
mod template;

#[cfg(test)]
mod tests;

pub use prefill::Prefill;
pub use service::{SkippedTemplate, TemplateCatalog, TemplateService};
pub use template::{Template, TemplateOrigin};
