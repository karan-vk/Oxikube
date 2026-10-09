//! Syntax colours: the tree-sitter parse made off the UI thread, and the styles of the rows on
//! screen read from it per frame (cached while the same rows stay on screen).

use std::ops::Range;
use std::sync::Arc;

use gpui::HighlightStyle;
use gpui_component::Rope;
use gpui_component::highlighter::{HighlightTheme, SyntaxHighlighter};

/// How many rows above and below the visible ones a style lookup covers, so a scroll of a few
/// rows reads the cache instead of the tree.
pub(crate) const OVERSCAN_ROWS: usize = 120;

/// A text parsed for highlighting. Made on a background thread ([`Parsed::parse`]); the UI
/// thread only queries it.
pub(crate) struct Parsed(SyntaxHighlighter);

impl Parsed {
    /// Parses `text` with the tree-sitter grammar of `language` (a full parse: run it off the UI
    /// thread).
    pub(crate) fn parse(text: &str, language: &str) -> Self {
        let rope = Rope::from(text);
        let mut highlighter = SyntaxHighlighter::new(language);
        highlighter.update(None, &rope, None);
        Self(highlighter)
    }

    /// The styles of bytes `range`, sorted and not overlapping.
    fn styles(
        &self,
        range: &Range<usize>,
        theme: &HighlightTheme,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        self.0.styles(range, theme)
    }
}

/// Styles read for one byte range under one theme.
struct Entry {
    range: Range<usize>,
    theme: usize,
    styles: Vec<(Range<usize>, HighlightStyle)>,
}

/// The styles last read: two ranges, because a frame asks for two (the `uniform_list` measures
/// its first row as well as drawing the visible ones), and one slot would make them evict each
/// other every frame once the view is scrolled.
#[derive(Default)]
pub(crate) struct StyleCache {
    entries: [Option<Entry>; 2],
    /// The slot the next miss replaces (the one used less recently).
    next: usize,
    /// How many times the parse was queried.
    #[cfg(test)]
    pub(crate) reads: usize,
}

impl StyleCache {
    /// The styles of `needed`, read again (over `wider`) only when no entry covers `needed`
    /// under `theme`.
    pub(crate) fn styles(
        &mut self,
        parsed: &Parsed,
        needed: &Range<usize>,
        wider: Range<usize>,
        theme: &Arc<HighlightTheme>,
    ) -> &[(Range<usize>, HighlightStyle)] {
        let theme_id = Arc::as_ptr(theme) as usize;
        let hit = self.entries.iter().position(|entry| {
            entry.as_ref().is_some_and(|entry| {
                entry.theme == theme_id
                    && entry.range.start <= needed.start
                    && needed.end <= entry.range.end
            })
        });
        let slot = match hit {
            Some(slot) => slot,
            None => {
                #[cfg(test)]
                {
                    self.reads += 1;
                }
                let slot = self.next;
                self.entries[slot] = Some(Entry {
                    styles: parsed.styles(&wider, theme),
                    range: wider,
                    theme: theme_id,
                });
                slot
            }
        };
        self.next = 1 - slot;
        self.entries[slot]
            .as_ref()
            .map_or(&[], |entry| entry.styles.as_slice())
    }

    /// Forgets the styles (a new text or a new parse).
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

/// The styles of `styles` (sorted, not overlapping) that fall in row `row`, relative to its start.
pub(crate) fn row_styles(
    styles: &[(Range<usize>, HighlightStyle)],
    row: Range<usize>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let first = styles.partition_point(|(range, _)| range.end <= row.start);
    styles[first..]
        .iter()
        .take_while(|(range, _)| range.start < row.end)
        .filter_map(|(range, style)| {
            let start = range.start.max(row.start) - row.start;
            let end = range.end.min(row.end) - row.start;
            (start < end).then_some((start..end, *style))
        })
        .collect()
}
