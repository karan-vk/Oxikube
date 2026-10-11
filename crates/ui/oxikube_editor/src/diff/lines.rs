//! [`ManifestDiff`]: a line diff of two texts (`similar`), laid out as the rows of an inline and
//! of a side-by-side view, with long unchanged stretches folded into gaps.

use std::time::Duration;

use serde_json::Value;
use similar::{DiffOp, TextDiff};

use super::canonical::normalized_text;

/// Unchanged lines kept around each change.
pub const CONTEXT_LINES: usize = 3;

/// How long the diff may search for the smallest edit before it settles for a larger one (a
/// 5 MB object with everything changed must not stall the background executor).
const DIFF_TIMEOUT: Duration = Duration::from_millis(500);

/// What a line of a diff is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// On both sides.
    Same,
    /// Only on the old side (the server's).
    Removed,
    /// Only on the new side (the buffer's).
    Added,
}

/// A row of the inline view: a line of either side, or a fold of unchanged lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineRow {
    /// A line; `old` / `new` are its zero-based numbers on each side it is on.
    Line {
        /// What the line is.
        kind: LineKind,
        /// Its line on the old side.
        old: Option<u32>,
        /// Its line on the new side.
        new: Option<u32>,
    },
    /// This many unchanged lines are not shown.
    Gap(u32),
}

/// A row of the side-by-side view: a line of each side (or none), or a fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideRow {
    /// A pair of lines; an empty side pads a line the other side added.
    Pair {
        /// The old side's line, with what it is (`Same` or `Removed`).
        old: Option<(u32, LineKind)>,
        /// The new side's line (`Same` or `Added`).
        new: Option<(u32, LineKind)>,
    },
    /// This many unchanged lines are not shown.
    Gap(u32),
}

/// A line diff of two texts, ready to draw either way. Plain data: built on the background
/// executor, shared with the view, reused by the apply preview (E10-S08) and the cross-cluster
/// diff (E20-S09), none of which needs the editor widget.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestDiff {
    old: Vec<String>,
    new: Vec<String>,
    inline: Vec<InlineRow>,
    side: Vec<SideRow>,
    hunks: usize,
}

impl ManifestDiff {
    /// The diff of `old` to `new`, line by line.
    pub fn between(old: &str, new: &str) -> Self {
        let diff = TextDiff::configure()
            .timeout(DIFF_TIMEOUT)
            .diff_lines(old, new);
        let groups = diff.grouped_ops(CONTEXT_LINES);
        let hunks = groups
            .iter()
            .filter(|group| group.iter().any(|op| !matches!(op, DiffOp::Equal { .. })))
            .count();
        let lines = |text: &str| text.lines().map(str::to_owned).collect::<Vec<_>>();
        let mut out = Self {
            old: lines(old),
            new: lines(new),
            inline: Vec::new(),
            side: Vec::new(),
            hunks,
        };
        if hunks == 0 {
            return out;
        }
        let mut shown_to = 0usize; // old lines before this are shown or folded
        for group in &groups {
            let first = group.first().map_or(0, |op| op.old_range().start);
            if first > shown_to {
                out.gap(first - shown_to);
            }
            for op in group {
                out.push_op(op);
            }
            shown_to = group.last().map_or(shown_to, |op| op.old_range().end);
        }
        if out.old.len() > shown_to {
            out.gap(out.old.len() - shown_to);
        }
        out
    }

    /// The diff of the live object to the buffer's document, both [normalised](super::normalize)
    /// (server fields ignored, keys sorted, Secret values fingerprinted) and written by the same
    /// writer. `live` is `None` when the object does not exist: every line is added.
    pub fn of_manifests(live: Option<&Value>, mine: &Value) -> Self {
        let old = live.map_or_else(String::new, |live| normalized_text(live, None));
        let new = normalized_text(mine, live);
        Self::between(&old, &new)
    }

    /// Whether the two sides are the same.
    pub fn is_empty(&self) -> bool {
        self.hunks == 0
    }

    /// How many separate changes (hunks) there are.
    pub fn hunks(&self) -> usize {
        self.hunks
    }

    /// The rows of the inline view.
    pub fn inline_rows(&self) -> &[InlineRow] {
        &self.inline
    }

    /// The rows of the side-by-side view.
    pub fn side_rows(&self) -> &[SideRow] {
        &self.side
    }

    /// Line `index` of the old side.
    pub fn old_line(&self, index: u32) -> &str {
        self.old.get(index as usize).map_or("", String::as_str)
    }

    /// Line `index` of the new side.
    pub fn new_line(&self, index: u32) -> &str {
        self.new.get(index as usize).map_or("", String::as_str)
    }

    /// Lines removed and added, for a summary ("2 changes · −1 +3").
    pub fn counts(&self) -> (usize, usize) {
        self.inline
            .iter()
            .fold((0, 0), |(removed, added), row| match row {
                InlineRow::Line {
                    kind: LineKind::Removed,
                    ..
                } => (removed + 1, added),
                InlineRow::Line {
                    kind: LineKind::Added,
                    ..
                } => (removed, added + 1),
                _ => (removed, added),
            })
    }

    fn gap(&mut self, lines: usize) {
        let lines = to_u32(lines);
        self.inline.push(InlineRow::Gap(lines));
        self.side.push(SideRow::Gap(lines));
    }

    fn push_op(&mut self, op: &DiffOp) {
        let (old, new) = (op.old_range(), op.new_range());
        if matches!(op, DiffOp::Equal { .. }) {
            for (o, n) in old.zip(new) {
                let (o, n) = (to_u32(o), to_u32(n));
                self.inline.push(InlineRow::Line {
                    kind: LineKind::Same,
                    old: Some(o),
                    new: Some(n),
                });
                self.side.push(SideRow::Pair {
                    old: Some((o, LineKind::Same)),
                    new: Some((n, LineKind::Same)),
                });
            }
            return;
        }
        for o in old.clone() {
            self.inline.push(InlineRow::Line {
                kind: LineKind::Removed,
                old: Some(to_u32(o)),
                new: None,
            });
        }
        for n in new.clone() {
            self.inline.push(InlineRow::Line {
                kind: LineKind::Added,
                old: None,
                new: Some(to_u32(n)),
            });
        }
        // Side by side, a replaced block pairs its lines row by row; the shorter side pads.
        let rows = old.len().max(new.len());
        for i in 0..rows {
            let o = (i < old.len()).then(|| (to_u32(old.start + i), LineKind::Removed));
            let n = (i < new.len()).then(|| (to_u32(new.start + i), LineKind::Added));
            self.side.push(SideRow::Pair { old: o, new: n });
        }
    }
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_texts_have_no_rows() {
        let diff = ManifestDiff::between("a\nb\n", "a\nb\n");
        assert!(diff.is_empty());
        assert!(diff.inline_rows().is_empty());
    }

    #[test]
    fn far_apart_changes_are_two_hunks_with_a_fold_between() {
        let old: String = (0..30).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l2\n", "L2\n").replace("l25\n", "L25\n");
        let diff = ManifestDiff::between(&old, &new);
        assert_eq!(diff.hunks(), 2);
        assert_eq!(diff.counts(), (2, 2));
        let gaps: Vec<u32> = diff
            .inline_rows()
            .iter()
            .filter_map(|row| match row {
                InlineRow::Gap(n) => Some(*n),
                InlineRow::Line { .. } => None,
            })
            .collect();
        // l6..l21 folded between the hunks, l29 after the last one's context.
        assert_eq!(gaps, vec![16, 1]);
        let pairs = diff
            .side_rows()
            .iter()
            .filter(|row| {
                matches!(
                    row,
                    SideRow::Pair {
                        old: Some((_, LineKind::Removed)),
                        new: Some((_, LineKind::Added))
                    }
                )
            })
            .count();
        assert_eq!(pairs, 2, "a replaced line sits beside its replacement");
        assert_eq!(diff.old_line(2), "l2");
        assert_eq!(diff.new_line(2), "L2");
    }
}
