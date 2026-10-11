//! [`ThreeWayMerge`]: the buffer and the live object merged line by line against the text the
//! buffer was loaded from (`similar`'s `TextMerge`). Changes on one side only are taken as they
//! are; changes on both sides to the same lines are conflicts, each settled by picking a side.
//!
//! The merge works on the buffer's own text, so the user's comments and layout survive it: the
//! base and the server's side are written by the same renderer, and only the lines the server
//! changed differ between them.
//!
//! The conflicts are also laid out as [`ConflictRow`]s, once, when the merge is made (off the UI
//! thread): rows of one height for a virtualised list, so a conflict thousands of lines long
//! costs what one screen of it costs to draw.

use std::ops::Range;

use similar::{MergeResolution, TextMerge};

/// The side a conflict is settled with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MergeSide {
    /// The buffer's lines ("Keep mine").
    Mine,
    /// The server's lines.
    Server,
}

/// A stretch of the merged text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeChunk {
    /// Lines both sides agree on (or only one side changed).
    Merged(String),
    /// Lines both sides changed differently.
    Conflict {
        /// The buffer's lines.
        mine: String,
        /// The server's lines.
        server: String,
        /// The side picked so far.
        chosen: Option<MergeSide>,
    },
}

/// One row of the conflicts laid out for a list: a conflict's header (a button per side), or a
/// line of each side, side by side. `chunk` indexes [`ThreeWayMerge::chunks`]; the ranges are
/// byte ranges of that conflict's `mine` and `server` text, without the newline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConflictRow {
    /// The head of conflict `conflict` (counting conflicts only).
    Header {
        /// The conflict's number, as [`ThreeWayMerge::choose`] takes it.
        conflict: usize,
        /// The conflict's chunk.
        chunk: usize,
    },
    /// The `n`th line of each side; a side shorter than the other has none.
    Line {
        /// The conflict's chunk.
        chunk: usize,
        /// The buffer's line.
        mine: Option<Range<usize>>,
        /// The server's line.
        server: Option<Range<usize>>,
    },
}

/// A three-way merge of the buffer and the server's object against the buffer's base. Plain data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreeWayMerge {
    chunks: Vec<MergeChunk>,
    rows: Vec<ConflictRow>,
}

impl ThreeWayMerge {
    /// Merges `mine` (the buffer) and `server` (the live object, rendered as the buffer was)
    /// against `base` (the text the buffer was loaded from).
    pub fn new(base: &str, mine: &str, server: &str) -> Self {
        let merge = TextMerge::from_lines(base, mine, server);
        let base_lines = |r: std::ops::Range<usize>| join(r.filter_map(|i| merge.base_line(i)));
        let mine_lines = |r: std::ops::Range<usize>| join(r.filter_map(|i| merge.ours_line(i)));
        let server_lines = |r: std::ops::Range<usize>| join(r.filter_map(|i| merge.theirs_line(i)));
        let mut chunks: Vec<MergeChunk> = Vec::new();
        for region in merge.regions() {
            let chunk = match region.resolution() {
                MergeResolution::Unchanged => MergeChunk::Merged(base_lines(region.base_range())),
                MergeResolution::Theirs => MergeChunk::Merged(server_lines(region.theirs_range())),
                MergeResolution::Conflict => MergeChunk::Conflict {
                    mine: mine_lines(region.ours_range()),
                    server: server_lines(region.theirs_range()),
                    chosen: None,
                },
                // `Ours`, `Both`, and any resolution a later `similar` adds: the buffer's lines.
                _ => MergeChunk::Merged(mine_lines(region.ours_range())),
            };
            // Neighbouring merged stretches read as one.
            match (chunks.last_mut(), chunk) {
                (Some(MergeChunk::Merged(text)), MergeChunk::Merged(more)) => text.push_str(&more),
                (_, chunk) => chunks.push(chunk),
            }
        }
        let rows = conflict_rows(&chunks);
        Self { chunks, rows }
    }

    /// The conflicts as rows of one height: a header per conflict, then its lines side by side.
    pub fn conflict_rows(&self) -> &[ConflictRow] {
        &self.rows
    }

    /// The text of a [`ConflictRow::Line`]'s side: the buffer's line (`Mine`) or the server's.
    pub fn conflict_line(&self, row: &ConflictRow, side: MergeSide) -> Option<&str> {
        let ConflictRow::Line {
            chunk,
            mine,
            server,
        } = row
        else {
            return None;
        };
        let Some(MergeChunk::Conflict {
            mine: mine_text,
            server: server_text,
            ..
        }) = self.chunks.get(*chunk)
        else {
            return None;
        };
        match side {
            MergeSide::Mine => mine_text.get(mine.clone()?),
            MergeSide::Server => server_text.get(server.clone()?),
        }
    }

    /// The side picked for the conflict in chunk `chunk`.
    pub fn chosen_at(&self, chunk: usize) -> Option<MergeSide> {
        match self.chunks.get(chunk)? {
            MergeChunk::Conflict { chosen, .. } => *chosen,
            MergeChunk::Merged(_) => None,
        }
    }

    /// The stretches, in order.
    pub fn chunks(&self) -> &[MergeChunk] {
        &self.chunks
    }

    /// How many conflicts there are.
    pub fn conflicts(&self) -> usize {
        self.chunks
            .iter()
            .filter(|c| matches!(c, MergeChunk::Conflict { .. }))
            .count()
    }

    /// How many conflicts have no side picked yet.
    pub fn unresolved(&self) -> usize {
        self.chunks
            .iter()
            .filter(|c| matches!(c, MergeChunk::Conflict { chosen: None, .. }))
            .count()
    }

    /// Picks `side` for conflict `index` (counting conflicts only). Returns whether there is
    /// such a conflict.
    pub fn choose(&mut self, index: usize, side: MergeSide) -> bool {
        let conflict = self
            .chunks
            .iter_mut()
            .filter_map(|c| match c {
                MergeChunk::Conflict { chosen, .. } => Some(chosen),
                MergeChunk::Merged(_) => None,
            })
            .nth(index);
        match conflict {
            Some(chosen) => {
                *chosen = Some(side);
                true
            }
            None => false,
        }
    }

    /// The merged text, once every conflict has a side; `None` before.
    pub fn text(&self) -> Option<String> {
        let mut out = String::new();
        for chunk in &self.chunks {
            match chunk {
                MergeChunk::Merged(text) => out.push_str(text),
                MergeChunk::Conflict {
                    mine,
                    server,
                    chosen,
                } => match (*chosen)? {
                    MergeSide::Mine => out.push_str(mine),
                    MergeSide::Server => out.push_str(server),
                },
            }
        }
        Some(out)
    }
}

/// Lines (each with its newline) as one text.
fn join<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    lines.collect()
}

/// The conflicts of `chunks` as [`ConflictRow`]s.
fn conflict_rows(chunks: &[MergeChunk]) -> Vec<ConflictRow> {
    let mut rows = Vec::new();
    let conflicts = chunks.iter().enumerate().filter_map(|(chunk, c)| match c {
        MergeChunk::Conflict { mine, server, .. } => Some((chunk, mine, server)),
        MergeChunk::Merged(_) => None,
    });
    for (conflict, (chunk, mine, server)) in conflicts.enumerate() {
        rows.push(ConflictRow::Header { conflict, chunk });
        let (mine, server) = (line_ranges(mine), line_ranges(server));
        for n in 0..mine.len().max(server.len()) {
            rows.push(ConflictRow::Line {
                chunk,
                mine: mine.get(n).cloned(),
                server: server.get(n).cloned(),
            });
        }
    }
    rows
}

/// The byte range of each line of `text`, without its line ending.
fn line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut start = 0;
    text.split_inclusive('\n')
        .map(|line| {
            let range = start..start + line.trim_end_matches(['\n', '\r']).len();
            start += line.len();
            range
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "a: 1\nb: 2\nc: 3\nd: 4\ne: 5\n";

    #[test]
    fn changes_to_different_lines_merge_and_keep_the_buffer_comments() {
        let mine = "# mine\na: 1\nb: 20\nc: 3\nd: 4\ne: 5\n";
        let server = "a: 1\nb: 2\nc: 3\nd: 4\ne: 50\n";
        let merge = ThreeWayMerge::new(BASE, mine, server);
        assert_eq!(merge.conflicts(), 0);
        assert_eq!(
            merge.text().as_deref(),
            Some("# mine\na: 1\nb: 20\nc: 3\nd: 4\ne: 50\n")
        );
    }

    #[test]
    fn the_same_line_changed_twice_is_a_conflict_settled_by_a_side() {
        let mine = "a: 1\nb: 20\nc: 3\nd: 4\ne: 5\n";
        let server = "a: 1\nb: 200\nc: 3\nd: 4\ne: 5\n";
        let mut merge = ThreeWayMerge::new(BASE, mine, server);
        assert_eq!((merge.conflicts(), merge.unresolved()), (1, 1));
        assert_eq!(merge.text(), None, "a conflict has no side yet");
        assert!(!merge.choose(1, MergeSide::Mine), "there is one conflict");
        assert!(merge.choose(0, MergeSide::Server));
        assert_eq!(merge.unresolved(), 0);
        assert_eq!(merge.text().as_deref(), Some(server));
        merge.choose(0, MergeSide::Mine);
        assert_eq!(merge.text().as_deref(), Some(mine));
    }

    #[test]
    fn conflicts_are_laid_out_as_a_header_and_a_row_per_line_pair() {
        let mine = "a: 1\nb: 20\nx: 1\nc: 3\nd: 4\ne: 5\n";
        let server = "a: 1\nb: 200\nc: 3\nd: 4\ne: 5\n";
        let mut merge = ThreeWayMerge::new(BASE, mine, server);
        assert_eq!(merge.conflicts(), 1);
        let rows = merge.conflict_rows().to_vec();
        assert!(matches!(rows[0], ConflictRow::Header { conflict: 0, .. }));
        assert_eq!(
            rows.len(),
            3,
            "a header, then two lines of mine beside one of the server's"
        );
        let line = |row, side| merge.conflict_line(row, side).map(str::to_owned);
        assert_eq!(line(&rows[1], MergeSide::Mine).as_deref(), Some("b: 20"));
        assert_eq!(line(&rows[1], MergeSide::Server).as_deref(), Some("b: 200"));
        assert_eq!(line(&rows[2], MergeSide::Mine).as_deref(), Some("x: 1"));
        assert_eq!(line(&rows[2], MergeSide::Server), None);
        let ConflictRow::Header { chunk, .. } = rows[0] else {
            unreachable!()
        };
        assert_eq!(merge.chosen_at(chunk), None);
        merge.choose(0, MergeSide::Server);
        assert_eq!(merge.chosen_at(chunk), Some(MergeSide::Server));
    }

    #[test]
    fn a_conflict_thousands_of_lines_long_is_one_row_per_line() {
        let base: String = (0..2_000).map(|i| format!("k{i}: {i}\n")).collect();
        let mine: String = (0..2_000).map(|i| format!("  k{i}:   {i}\n")).collect();
        let server = base.replace("k1000: 1000\n", "k1000: changed\n");
        let merge = ThreeWayMerge::new(&base, &mine, &server);
        assert_eq!(merge.conflicts(), 1);
        assert_eq!(merge.conflict_rows().len(), 1 + 2_000);
    }
}
