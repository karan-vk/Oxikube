//! Writing log text as a [`ContextBlock`] an agent can cite: a `# key: value` header that says
//! where the lines came from, the notes about what is missing, then the lines.

use jiff::Timestamp;
use oxikube_domain::agent::{ContextBlock, MAX_CONTEXT_BLOCK_BYTES};

use crate::context::pending::ContextSource;

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
    out.push_str(&format!(
        "# cluster: {} ({})\n",
        source.cluster_name, source.cluster
    ));
    out.push_str(&format!("# namespace: {}\n", source.namespace));
    out.push_str(&format!("# source: {}\n", source.subject));
    if let Some(container) = &source.container {
        out.push_str(&format!("# container: {container}\n"));
    }
    if let Some((first, last)) = source.span {
        out.push_str(&format!("# time: {} to {}\n", utc(first), utc(last)));
    }
    out.push_str(&format!("# lines: {}\n", source.lines));
    out.push_str("# secrets are masked on a best-effort basis\n");
    out
}

/// `2026-10-07T12:00:00.123Z`.
fn utc(at: Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
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
        body.push_str("# note: ");
        body.push_str(note);
        body.push('\n');
    }
    body.push_str(lines);
    let mut block = ContextBlock::bounded(title, "text/plain", body);
    block.truncated |= cut;
    block
}
