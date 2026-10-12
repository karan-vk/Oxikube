//! Bulk actions on a selection (E12-S19): the pure parts behind `selection::Delete`,
//! `selection::Restart` and `selection::Label`.
//!
//! | File | Holds |
//! |---|---|
//! | `summary` | [`SelectionSummary`]: how many objects of each kind in each namespace, for the one confirmation a bulk action asks |
//! | `labels` | [`LabelField`], [`LabelEdit`]: the label editor's edits, their validation, the merge patch they make and the preview of the result |
//!
//! Nothing here sends anything: the app's `selection` module runs the commands and the UI draws
//! these types.

mod labels;
mod summary;

pub use labels::{
    LabelEdit, LabelField, LabelPreview, MAX_EDITS, label_merge_patch, preview_edits,
    selector_warning, validate_edits,
};
pub use summary::{GroupCount, SelectionSummary};
