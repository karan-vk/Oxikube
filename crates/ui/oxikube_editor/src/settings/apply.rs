//! [`apply_changes`]: pushes the editor settings that changed into one open code editor, through
//! [`EditorApi`] only.
//!
//! Only the fields that differ from what was applied last are touched, so a change of `tab_size`
//! leaves the line wrapping the user toggled in that editor alone. None of it replaces the buffer:
//! the text, the selections and the undo history stay.

use oxikube_ui::editor::EditorApi;

use super::EditorSettings;

/// Applies to `editor` what differs between `previous` (what it was last given; `None` for an
/// editor that was never given any) and `next`.
///
/// The settings that are not about the buffer's look (`validation`, `field_manager`,
/// `show_managed_fields`, `mask_secrets`) are read where they are used and not applied here.
pub fn apply_changes(
    previous: Option<&EditorSettings>,
    next: &EditorSettings,
    editor: &mut dyn EditorApi,
) {
    if previous.is_none_or(|p| p.soft_wrap != next.soft_wrap) {
        editor.set_soft_wrap(next.soft_wrap);
    }
    if previous.is_none_or(|p| p.tab_size != next.tab_size) {
        editor.set_tab_size(next.tab_size);
    }
    if previous.is_none_or(|p| p.font_family != next.font_family || p.font_size != next.font_size) {
        editor.set_font(next.font());
    }
}
