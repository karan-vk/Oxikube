//! [`values_diff`]: the line diff the rollback preview shows between the values now and the
//! values of the revision to restore.

use similar::{ChangeTag, TextDiff};

/// How a line of the diff relates to the two texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    /// In both.
    Same,
    /// Only in the values now: rolling back removes it.
    Removed,
    /// Only in the revision to restore: rolling back brings it back.
    Added,
}

/// One line of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// Which side has it.
    pub kind: DiffKind,
    /// The line, without its line break.
    pub text: String,
}

/// The lines that differ between `now` and `target` (the values of the revision to restore),
/// each with a line of context on both sides; empty when the texts are equal. Callers pass text
/// that is already masked: this never unmasks anything.
pub fn values_diff(now: &str, target: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(now, target);
    if diff.ratio() >= 1.0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for group in diff.grouped_ops(1) {
        if !lines.is_empty() {
            lines.push(DiffLine {
                kind: DiffKind::Same,
                text: "...".to_owned(),
            });
        }
        for op in group {
            for change in diff.iter_changes(&op) {
                let kind = match change.tag() {
                    ChangeTag::Equal => DiffKind::Same,
                    ChangeTag::Delete => DiffKind::Removed,
                    ChangeTag::Insert => DiffKind::Added,
                };
                lines.push(DiffLine {
                    kind,
                    text: change.value().trim_end_matches(['\n', '\r']).to_owned(),
                });
            }
        }
    }
    lines
}
