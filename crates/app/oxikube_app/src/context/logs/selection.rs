//! A selection in the log viewer as agent context: "Send to agent".

use oxikube_domain::agent::MAX_CONTEXT_BLOCK_BYTES;
use oxikube_domain::redact::redact;

use super::block::{block, header, lines_budget};
use crate::context::pending::{ContextSource, QueuedContext};

/// The block for the lines the user selected, written as the viewer shows them (`text`, one line
/// per line), with `source` as its header so the agent can cite where they came from.
///
/// The text is masked of secrets here, so what reaches the agent is not what a file save or a copy
/// would hold (those are the user's own, unmasked). Over the block budget the first lines are kept
/// and the rest is counted in a note.
///
/// `source.lines` is the number of lines in `text`; `unread` is how many more lines were selected
/// but never made it into `text` (the caller's read limit), so the note counts them too.
pub fn selection_context(mut source: ContextSource, text: &str, unread: usize) -> QueuedContext {
    let masked = redact(text);
    let budget = lines_budget(MAX_CONTEXT_BLOCK_BYTES);
    let (kept, cut) = head_lines(&masked, budget);
    let left_out = cut.saturating_add(unread);
    let notes: Vec<String> = (left_out > 0)
        .then(|| {
            format!(
                "{left_out} further selected lines were left out to fit {} KiB.",
                MAX_CONTEXT_BLOCK_BYTES / 1024
            )
        })
        .into_iter()
        .collect();
    source.lines = source.lines.saturating_sub(cut);
    let title = format!(
        "Logs {}/{} ({} lines)",
        source.namespace, source.subject, source.lines
    );
    let block = block(title, &header(&source), &notes, kept, left_out > 0);
    QueuedContext { block, source }
}

/// The longest prefix of `text` of whole lines within `budget` bytes, and how many lines follow it.
fn head_lines(text: &str, budget: usize) -> (&str, usize) {
    if text.len() <= budget {
        return (text, 0);
    }
    let cut = text.as_bytes()[..budget]
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |ix| ix + 1);
    let rest = &text[cut..];
    (&text[..cut], rest.lines().count())
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use oxikube_domain::ids::{ClusterId, ContextName};

    use super::*;

    fn source(lines: usize) -> ContextSource {
        ContextSource {
            cluster: ClusterId::new("~/.kube/config", &ContextName::new("kind")),
            cluster_name: "kind".into(),
            namespace: "default".into(),
            subject: "web-0".into(),
            container: Some("app".into()),
            span: Some((
                Timestamp::from_second(1_760_000_000).unwrap(),
                Timestamp::from_second(1_760_000_060).unwrap(),
            )),
            lines,
        }
    }

    #[test]
    fn the_block_carries_the_source_the_agent_can_cite() {
        let item = selection_context(source(2), "a\nb\n", 0);
        let body = &item.block.body;
        for needle in [
            "# cluster: kind (",
            "# namespace: default",
            "# source: web-0",
            "# container: app",
            "# time: 2025-10-09T08:53:20.000Z to 2025-10-09T08:54:20.000Z",
            "# lines: 2",
        ] {
            assert!(body.contains(needle), "{needle}\n{body}");
        }
        assert!(body.ends_with("a\nb\n"));
        assert_eq!(item.block.title, "Logs default/web-0 (2 lines)");
        assert!(!item.block.truncated);
        assert_eq!(item.source.lines, 2);
    }

    #[test]
    fn secrets_are_masked_before_the_agent_sees_them() {
        let item = selection_context(
            source(1),
            "token=abcdef0123456789 Bearer s3cr3tvalue123456\n",
            0,
        );
        assert!(!item.block.body.contains("abcdef0123456789"));
        assert!(!item.block.body.contains("s3cr3tvalue123456"));
    }

    #[test]
    fn an_oversized_selection_keeps_its_first_lines_and_says_so() {
        let text: String = (0..2_000).map(|i| format!("{i:0>100}\n")).collect();
        let item = selection_context(source(2_000), &text, 0);
        assert!(item.block.body.len() <= MAX_CONTEXT_BLOCK_BYTES);
        assert!(item.block.truncated);
        assert!(
            item.block
                .body
                .contains("further selected lines were left out")
        );
        assert!(item.block.body.contains(&format!("{:0>100}\n", 0)));
        assert!(!item.block.body.contains(&format!("{:0>100}\n", 1_999)));
        assert!(item.source.lines < 2_000);
    }

    #[test]
    fn lines_the_caller_never_read_are_counted_in_the_note() {
        let item = selection_context(source(2), "a\nb\n", 1_230);
        assert!(
            item.block
                .body
                .contains("1230 further selected lines were left out"),
            "{}",
            item.block.body
        );
        assert!(item.block.truncated);
        assert_eq!(item.source.lines, 2, "the lines the block holds");
        assert!(item.block.title.contains("(2 lines)"));
    }

    #[test]
    fn the_note_adds_the_unread_lines_to_the_ones_cut_for_the_budget() {
        let text: String = (0..2_000).map(|i| format!("{i:0>100}\n")).collect();
        let item = selection_context(source(2_000), &text, 500);
        let kept = item.source.lines;
        let expected = format!("{} further selected lines", 2_000 - kept + 500);
        assert!(item.block.body.contains(&expected), "{expected}");
    }
}
