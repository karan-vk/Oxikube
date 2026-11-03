//! The keymap editor page (E21-S07): a searchable table of every action, its context, its
//! binding and where the binding comes from, including the commands nothing is bound to.
//!
//! Open it from the palette ("Keymap Editor"), the sidebar's Settings > Keymap entry or an
//! agent's `keymap.show_editor`: `keymap::ShowEditor` shows it as a tab of the active cluster's
//! tab. The page is the machine's, not a cluster's, so a cluster tab is only a place to live.
//!
//! | Module | Holds |
//! |---|---|
//! | `search` | [`Query`]: words matched against title, action id, context and keystrokes (`Ctrl+D` finds `ctrl-d`) |
//! | `model` | [`KeymapListModel`]: the rows, the search, the context filter, the "unbound" switch and the empty states, as data |
//! | `delegate` | [`KeymapDelegate`]: the virtualised table (`oxikube_ui::table`) over the model |
//! | `view`, `render` | [`KeymapEditorView`]: the page, its toolbar and its workspace item |
//! | `sidebar` | the Settings section's "Keymap" entry |
//! | `command` | [`register_commands`], [`KeymapEditorRequestSink`], [`KeymapEditorViews`]: `keymap::ShowEditor`, `keymap::RecordBinding` |
//! | `edits` | [`KeymapEditSink`], [`KeymapEdits`]: `keymap::SetBinding` and `keymap::RemoveBinding`, applied on the UI thread by `oxikube_keymap::edit` (E21-S08) |
//! | `editing` | the page's Record and Remove buttons (E21-S08) |
//! | [`recorder`] | [`KeybindRecorder`](recorder::KeybindRecorder): the modal that records keys, shows conflicts and saves (E21-S08) |
//!
//! The rows come from [`oxikube_keymap::keymap_rows`], built from the layers' files so that a
//! binding the user replaced or `null`led stays visible ("overridden", "unbound by you"); the
//! page re-reads them on [`oxikube_keymap::subscribe_changes`], keeping the search, filters,
//! selection and scroll. "Open keymap.json" is `keymap::OpenUser`.
//!
//! Editing (E21-S08): "Record binding" (`keymap::RecordBinding`) opens the [`recorder`] on the
//! selected row; its Save is `keymap::SetBinding`. "Remove binding" is `keymap::RemoveBinding`: a
//! binding of the user's file is deleted, a default one unbound with `null` there. Both commands
//! edit `keymap.json` through `oxikube_keymap::edit` (comments kept, validated before writing,
//! in force at once) and never touch a default keymap.

mod command;
mod delegate;
mod editing;
mod edits;
mod model;
pub mod recorder;
mod render;
mod search;
mod sidebar;
mod view;

#[cfg(test)]
mod tests;

pub use command::{
    KeymapEditorRequest, KeymapEditorRequestSink, KeymapEditorViews, KeymapEditorViewsDeps,
    register_commands,
};
pub use delegate::{KeymapDelegate, context_label, default_file_name, source_label};
pub use edits::{KeymapEditRequest, KeymapEditSink, KeymapEdits};
pub use model::{ContextFilter, EmptyState, KeymapListModel, RowKey};
pub use search::{Haystack, Query, canonical_keystroke};
pub use sidebar::{ENTRY_ID, SECTION_ID, entry, register_sidebar, section};
pub use view::{ITEM_KEY, KeymapEditorDeps, KeymapEditorView};
