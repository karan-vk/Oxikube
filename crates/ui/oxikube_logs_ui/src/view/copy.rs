//! Copy (k9s `c`, `logs::Copy`): the selected lines, else the lines on screen, to the clipboard.

use std::ops::Range;

use gpui::{ClipboardItem, Context};
use oxikube_app::logs::export::copy_text;
use oxikube_workspace::Toast;

use super::LogView;
use super::text::{group, lines_of};

/// The most a copy puts on the clipboard: building and handing over more than this does not fit
/// a frame. A bigger selection copies its first lines and says so; saving it to a file has no
/// such limit.
pub const COPY_LIMIT_BYTES: usize = 5 * 1024 * 1024;

impl LogView {
    /// The seqs a copy takes: the selection, else what is on screen.
    pub(crate) fn copy_seqs(&self) -> Option<Range<u64>> {
        match self.selection.range() {
            Some(range) => Some(*range.start()..range.end() + 1),
            None => self.viewport_seqs(),
        }
    }

    /// Copies the selected lines (raw text, with the timestamp when the view shows it), or with no
    /// selection the lines on screen, up to [`COPY_LIMIT_BYTES`]. A toast says how many lines went
    /// to the clipboard, or that the copy was cut.
    pub fn copy_lines(&mut self, cx: &mut Context<Self>) {
        let (Some(session), Some(seqs)) = (self.session.as_ref(), self.copy_seqs()) else {
            self.toast(Toast::info("There are no lines to copy."), cx);
            return;
        };
        let spec = self.spec_for(seqs, self.export_format());
        let copied = copy_text(&session.reader(), &spec, COPY_LIMIT_BYTES);
        if copied.lines == 0 {
            self.toast(Toast::info("There are no lines to copy."), cx);
            return;
        }
        let lines = copied.lines;
        let truncated = copied.truncated;
        cx.write_to_clipboard(ClipboardItem::new_string(copied.text));
        let toast = if truncated {
            Toast::warning(format!(
                "Copied the first {} lines: a copy is limited to 5 MB. Save to a file to keep the rest.",
                group(lines)
            ))
        } else {
            Toast::success(format!("Copied {}", lines_of(lines)))
        };
        self.toast(toast.key("logs-copy"), cx);
    }
}
