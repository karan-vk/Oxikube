//! Text input: a single-line [`Input`] and a multi-line [`Textarea`].
//!
//! Editor glue (the YAML/log/terminal views) lives in `oxikube_editor`, which builds on this
//! module's state types.

pub use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};

/// The text-editing actions inputs handle, for the Edit menu and key bindings.
pub mod actions {
    pub use gpui_component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
}
