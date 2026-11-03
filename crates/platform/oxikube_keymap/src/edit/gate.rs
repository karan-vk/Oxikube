//! The check before a write: the edited text goes through the one validator
//! ([`crate::validate()`]), and the write is refused when the edit adds an error.
//!
//! Errors the file already had are not the edit's doing, so they do not block it (the user can
//! still rebind a key while an unrelated line is wrong); warnings never block.

use super::ops::{EditError, KeybindUpdateOperation, apply_operations};
use crate::diagnostics::{KeymapDiagnostic, Severity};
use crate::known::KnownNames;
use crate::registry::ActionRegistry;
use crate::validate::validate;

/// What an edit would write, and what the validator says about it.
#[derive(Clone, Debug, PartialEq)]
pub struct EditPreview {
    /// The text after the edit.
    pub text: String,
    /// Every problem of that text, in file order.
    pub diagnostics: Vec<KeymapDiagnostic>,
    /// The errors of that text the original did not have: the write is refused while any.
    pub introduced: Vec<KeymapDiagnostic>,
}

impl EditPreview {
    /// Whether the edit may be written.
    pub fn is_writable(&self) -> bool {
        self.introduced.is_empty()
    }

    /// The warnings of the edited text, for the recorder to show.
    pub fn warnings(&self) -> impl Iterator<Item = &KeymapDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity() == Severity::Warning)
    }
}

/// The errors of `after` that `before` did not have, compared by what they say (positions move).
pub fn introduced_errors(
    before: &[KeymapDiagnostic],
    after: &[KeymapDiagnostic],
) -> Vec<KeymapDiagnostic> {
    let key = |d: &KeymapDiagnostic| (d.code(), d.keystrokes.clone(), d.message());
    let mut old: Vec<_> = before
        .iter()
        .filter(|d| d.severity() == Severity::Error)
        .map(key)
        .collect();
    after
        .iter()
        .filter(|d| d.severity() == Severity::Error)
        .filter(|d| {
            let k = key(d);
            match old.iter().position(|o| *o == k) {
                Some(at) => {
                    old.swap_remove(at);
                    false
                }
                None => true,
            }
        })
        .cloned()
        .collect()
}

/// Apply `operations` to `text` and validate the result against `actions` and `known`. Pure, so
/// the recorder runs it on the background executor while the user types, and the write path
/// runs it again on the file as it is on disk.
///
/// # Errors
///
/// When the operations cannot be applied ([`EditError`]).
pub fn preview_edit(
    text: &str,
    operations: &[KeybindUpdateOperation],
    actions: &ActionRegistry,
    known: &KnownNames,
) -> Result<EditPreview, EditError> {
    let edited = apply_operations(text, operations)?;
    let diagnostics = validate(&edited, actions, known);
    let before = validate(text, actions, known);
    let introduced = introduced_errors(&before, &diagnostics);
    Ok(EditPreview {
        text: edited,
        diagnostics,
        introduced,
    })
}
