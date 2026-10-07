//! Writing log text as a [`ContextBlock`] an agent can cite: a `# key: value` header that says
//! where the lines came from, the notes about what is missing, then the lines.

use std::fmt::Write as _;

use oxikube_domain::agent::{ContextBlock, MAX_CONTEXT_BLOCK_BYTES};

use crate::context::pending::ContextSource;
use crate::logs::export::utc_millis;

/// Bytes kept back from the budget for the header and the notes.
pub(super) const RESERVE_BYTES: usize = 2_048;

/// The budget for the lines themselves when the whole block may be `cap` bytes.
pub(super) fn lines_budget(cap: usize) -> usize {
    let cap = cap.min(MAX_CONTEXT_BLOCK_BYTES);
    cap.saturating_sub(RESERVE_BYTES.min(cap / 2)).max(1)
}

/// The header of a block: `# cluster: ...` and the rest of `source`.
pub(super) fn header(source: &ContextSource) -> String {
    let mut out = String::new();
    // Writing to a `String` cannot fail.
    let _ = writeln!(
        out,
        "# cluster: {} ({})",
        source.cluster_name, source.cluster
    );
    let _ = writeln!(out, "# namespace: {}", source.namespace);
    let _ = writeln!(out, "# source: {}", source.subject);
    if let Some(container) = &source.container {
        let _ = writeln!(out, "# container: {container}");
    }
    if let Some((first, last)) = source.span {
        let _ = writeln!(out, "# time: {} to {}", utc_millis(first), utc_millis(last));
    }
    let _ = writeln!(out, "# lines: {}", source.lines);
    out.push_str("# secrets are masked on a best-effort basis\n");
    out
}

/// A block titled `title` holding `header`, one `# note:` line per note, then `lines`. The block
/// is flagged truncated when `cut` says lines were left out; the notes say how.
pub(super) fn block(
    title: String,
    header: &str,
    notes: &[String],
    lines: &str,
    cut: bool,
) -> ContextBlock {
    let mut body = String::with_capacity(header.len() + lines.len() + 256);
    body.push_str(header);
    for note in notes {
        let _ = writeln!(body, "# note: {note}");
    }
    body.push_str(lines);
    let mut block = ContextBlock::bounded(title, "text/plain", body);
    block.truncated |= cut;
    block
}
