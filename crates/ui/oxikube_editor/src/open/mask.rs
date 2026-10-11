//! The rule for a masked Secret that is still masked when the buffer is applied.
//!
//! A Secret opened with its values masked has [`MASKED`](super::MASKED) where each value was. The
//! placeholder must never reach the cluster as the new value, so each one left in the buffer is an
//! error diagnostic: the problem count then blocks the apply (E10-S08 refuses a buffer with
//! errors), and the squiggle says what to do. Revealing the values (or typing new ones) clears
//! them. Unchanged keys are not "merged back in": the buffer is the manifest, as everywhere in the
//! editor.

use oxikube_ui::editor::{DiagnosticLevel, EditorDiagnostic};

use super::regions::placeholders;
use crate::yaml::ParseResult;

/// The code of the diagnostic on a placeholder.
pub const MASKED_CODE: &str = "masked-secret-value";

/// One error per placeholder in the Secrets of `parsed`.
pub fn masked_diagnostics(parsed: &ParseResult) -> Vec<EditorDiagnostic> {
    let mut found = Vec::new();
    for doc in parsed.docs() {
        for region in placeholders(doc, parsed.text()) {
            found.push(
                EditorDiagnostic::new(
                    region.range,
                    DiagnosticLevel::Error,
                    format!(
                        "`{}` is still masked: reveal the values or replace it before applying",
                        region.key
                    ),
                )
                .with_code(MASKED_CODE),
            );
        }
    }
    found.sort_by_key(|d| d.range.start);
    found
}
