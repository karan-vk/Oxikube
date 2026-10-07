//! OSC 8 hyperlinks in a [`TerminalSnapshot`] (E09-S05): which cells carry a link, and the URIs
//! interned across frames so a link that stays on screen is not copied every frame.

use std::sync::Arc;

use super::TerminalSnapshot;

/// URIs a snapshot keeps between frames before it starts over (a screen of distinct links).
const MAX_KEPT_URIS: usize = 256;

/// The index of `uri` in `uris`, added when missing. A steady screen of links allocates nothing.
fn intern(uris: &mut Vec<Arc<str>>, uri: &str) -> u16 {
    let index = match uris.iter().position(|known| **known == *uri) {
        Some(index) => index,
        None => {
            uris.push(Arc::from(uri));
            uris.len() - 1
        }
    };
    // `MAX_KEPT_URIS` bounds the list at the start of a frame; one frame adds at most a screen.
    u16::try_from(index).unwrap_or(u16::MAX)
}

/// Forgets the previous frame's links (keeping the interned URIs unless there are too many).
pub(super) fn start_frame(out: &mut TerminalSnapshot) {
    out.hyperlinks.clear();
    if out.hyperlink_uris.len() > MAX_KEPT_URIS {
        out.hyperlink_uris.clear();
    }
}

/// Records that cell `index` links to `uri`. Cells arrive in order, so `hyperlinks` stays sorted.
pub(super) fn record(out: &mut TerminalSnapshot, index: usize, uri: &str) {
    let uri = intern(&mut out.hyperlink_uris, uri);
    out.hyperlinks.push((index, uri));
}

impl TerminalSnapshot {
    /// The OSC 8 hyperlink of the cell at viewport `row`, `column`, if it has one.
    pub fn hyperlink_at(&self, row: usize, column: usize) -> Option<&Arc<str>> {
        if column >= self.columns {
            return None;
        }
        let index = row * self.columns + column;
        let found = self
            .hyperlinks
            .binary_search_by_key(&index, |&(cell, _)| cell)
            .ok()?;
        self.hyperlink_uris
            .get(usize::from(self.hyperlinks[found].1))
    }
}
