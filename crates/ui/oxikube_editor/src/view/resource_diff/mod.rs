//! The cross-cluster resource diff tab (E20-S09): `resource::DiffWith`.
//!
//! | File | Holds |
//! |---|---|
//! | `data` | [`DiffData`] (the app's [`ResourceDiff`](oxikube_app::diff::ResourceDiff) plus the line diff of the two sides' YAML, built off the UI thread), [`DiffState`] |
//! | `command` | [`register_diff_command`]: the bus handler that reads both objects through `oxikube_app::diff::DiffService`, shows progress in the tab, and returns the changed-path summary to an agent |
//! | `view` | [`ResourceDiffView`]: the tab (a workspace item), its switches (status, name and namespace, layout) |
//! | `render` | the header, the changed-path summary and the line diff, both virtualised |
//! | `picker` | [`DiffTargetDialog`]: the "Diff with..." picker for the second object |
//! | `open` | the [`EditorViews`](super::EditorViews) side: the picker and the tab |
//!
//! The line diff is the editor's (`ManifestDiff`, the diff vs live of E10-S07): comment- and
//! key-order-insensitive, a side that does not exist is empty so the other side reads all added.
//! A Secret's values are fingerprints in the text and `changed` / `same` per key in the summary;
//! nothing here writes an object or a value anywhere, and the tab is not saved with the layout.

mod command;
mod data;
mod open;
mod picker;
mod render;
#[cfg(test)]
mod tests;
mod view;

pub use command::register_diff_command;
pub use data::{DiffData, DiffState};
pub use picker::DiffTargetDialog;
pub use view::ResourceDiffView;
