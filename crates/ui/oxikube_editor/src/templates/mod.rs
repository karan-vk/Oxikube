//! Create from template (E10-S09): the embedded manifests of the core kinds, and the "Create
//! resource" picker that opens one in a new manifest editor.
//!
//! ```text
//! resource::Create ─> EditorViews ─TemplateService::catalog (spawn_kube)─> TemplatePicker
//!   (palette, the table's "New", the keymap)   embedded + ~/.config/oxikube/templates/*.yaml
//!                                                    │ Enter / click
//!                                                    ▼
//!                       ManifestEditor tab "New Deployment", the cluster's selected namespace
//!                       filled in, no live object (the diff compares with nothing)
//!                                                    │ cmd-s: one document or several
//!                                                    ▼
//!                       dry run of each, one combined preview, apply in order, results panel
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `registry` | [`embedded`]: one minimal manifest per core kind (`templates/*.yaml`, compiled in) |
//! | `picker` | [`TemplatePicker`]: a generic [`Picker`](oxikube_palette::Picker) over [`TemplatePickerDelegate`], searchable by kind or template name |
//!
//! The service that merges the user's templates and fills in the namespace is
//! `oxikube_app::TemplateService` (no UI types); the embedded files live here, in the UI crate,
//! and are passed to it. The templates keep `namespace: default` where the namespace goes, so
//! they are valid YAML before anything is substituted.

mod picker;
mod registry;

pub use picker::{TemplatePicker, TemplatePickerDelegate, TemplateSink};
pub use registry::embedded;
