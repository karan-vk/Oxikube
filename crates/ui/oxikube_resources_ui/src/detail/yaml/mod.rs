//! The YAML tab (E07-S06): the object as read-only, highlighted YAML, secrets masked.
//!
//! | File | Holds |
//! |---|---|
//! | `text` | [`yaml_text`], [`YamlOptions`]: the displayed text as a pure function over the object (managedFields, Secret masking) |
//! | `tab` | [`YamlTab`] and the view's methods: the text made off the UI thread per object version, the managedFields toggle, the copy and save texts |
//! | `render` | the toolbar (managedFields, copy, save) and the body: an `oxikube_ui::code_view::CodeView` (read-only, virtualised, tree-sitter coloured), under a skeleton until it is laid out |
//!
//! The text is made when the tab is shown and again only when the object's version (or the
//! managedFields choice) changes, on the background executor (E07-P598): the object goes there
//! as the `Arc` the view already holds, is copied, stripped, masked and serialised there, and the
//! text comes back as an `Arc<str>` the code view shares (it lays its rows out and parses it off
//! the UI thread as well, and keeps its scroll). The text on screen stays until the new one is
//! ready. What is displayed is exactly what copy and save write, so a masked Secret is copied and
//! saved masked. The cached object is never mutated: the text is made from a copy.
//!
//! The toolbar buttons send commands (`resource::ToggleManagedFields`, `resource::CopyYaml`,
//! `resource::SaveYaml`) like the palette and an agent do; [`ResourceViews`] applies them to the
//! open detail on the UI thread.
//!
//! [`ResourceViews`]: crate::ResourceViews

mod render;
mod tab;
mod text;

#[cfg(test)]
mod tests;

pub(super) use tab::YamlTab;
pub use text::{YamlOptions, has_managed_fields, yaml_text};
