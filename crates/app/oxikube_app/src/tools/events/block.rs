//! Events as the text of a [`ContextBlock`]: a `# key: value` header that says where they came
//! from, the notes about what is missing, then one line per event, newest first.
//!
//! `@events` and "Send to agent" write the same block. It is cut to the caller's byte budget by
//! dropping the **oldest** events whole, so the same events in the same budget always give the
//! same text, and a `# note:` line says how many were left out and how to get them.

use std::sync::Arc;

use oxikube_domain::agent::{ContextBlock, MAX_CONTEXT_BLOCK_BYTES};
use oxikube_domain::event::Event;

use super::bounds::MAX_BLOCK_MESSAGE_CHARS;
use super::render::block_line;

/// Bytes kept back from the budget for the closing note.
const FOOTER_RESERVE: usize = 192;

/// What a block says.
pub(super) struct EventsBlock<'a> {
    /// The block's label, `Events Pod shop/web-0`.
    pub(super) title: String,
    /// `# cluster: ...`-style lines, without the trailing newline.
    pub(super) header: Vec<String>,
    /// Sentences about what is missing or may be stale; each becomes a `# note:` line.
    pub(super) notes: Vec<String>,
    /// The events to quote, newest first.
    pub(super) events: &'a [Arc<Event>],
    /// How many events matched in all (more than `events` when the caller capped the list).
    pub(super) total: usize,
}

impl EventsBlock<'_> {
    /// The block, its body within `cap` bytes.
    pub(super) fn build(&self, cap: usize) -> ContextBlock {
        let cap = cap.min(MAX_CONTEXT_BLOCK_BYTES);
        let mut head = String::new();
        for line in &self.header {
            head.push_str(line);
            head.push('\n');
        }
        head.push_str("# events are quoted newest first, by last seen; secrets are masked on a best-effort basis\n");
        for note in &self.notes {
            head.push_str("# note: ");
            head.push_str(note);
            head.push('\n');
        }
        let budget = cap.saturating_sub(head.len() + FOOTER_RESERVE);
        let mut body = head;
        let mut shown = 0;
        let mut used = 0;
        for event in self.events {
            let line = block_line(event, MAX_BLOCK_MESSAGE_CHARS);
            if used + line.len() + 1 > budget {
                break;
            }
            body.push_str(&line);
            body.push('\n');
            used += line.len() + 1;
            shown += 1;
        }
        let left_out = self.total.saturating_sub(shown);
        if left_out > 0 {
            body.push_str(&format!(
                "# note: {left_out} older event{} left out ({shown} of {} shown); ask for a \
                 narrower selection or use the k8s.get_events tool with `since` or `involved`\n",
                if left_out == 1 { "" } else { "s" },
                self.total,
            ));
        }
        if self.total == 0 {
            body.push_str("# no events are held for this selection\n");
        }
        let mut block = ContextBlock::bounded(self.title.clone(), "text/plain", body);
        block.truncated |= left_out > 0;
        block
    }
}
