//! Choosing the lines of an excerpt and writing them as redacted text within a byte budget.

use oxikube_domain::redact::redact;

use super::super::export::ExportFormat;
use super::super::{LogBuffer, LogEntry, LogMatcher};

/// The lines picked out of a buffer, oldest first, and what the pick left out.
pub(super) struct Picked {
    /// The newest matching lines, at most the tail.
    pub entries: Vec<LogEntry>,
    /// Lines the buffer retained (all of them were tested).
    pub scanned: usize,
    /// Lines that matched.
    pub matched: usize,
    /// Lines the buffer dropped before they could be read.
    pub buffer_dropped: u64,
}

/// The newest `tail` lines of `buffer` that `matcher` accepts, oldest first.
pub(super) fn pick(buffer: &LogBuffer, matcher: &LogMatcher, tail: usize) -> Picked {
    let mut newest_first = Vec::with_capacity(tail.min(buffer.len()));
    let mut matched = 0usize;
    for entry in buffer.iter().rev() {
        if !matcher.matches(&entry.text) {
            continue;
        }
        matched += 1;
        if newest_first.len() < tail {
            newest_first.push(entry.clone());
        }
    }
    newest_first.reverse();
    Picked {
        entries: newest_first,
        scanned: buffer.len(),
        matched,
        buffer_dropped: buffer.dropped(),
    }
}

/// Lines written as text.
pub(super) struct Rendered {
    /// `timestamp pod/container text` per line, secrets masked, at most the budget.
    pub text: String,
    /// Lines in `text`.
    pub lines: usize,
    /// Whether older lines were dropped to fit the budget.
    pub budget_cut: bool,
}

/// Writes `entries` (oldest first) in `format`, keeping the newest lines that fit `budget` bytes.
///
/// The text is redacted as a whole, after the lines are joined, so a secret that spans lines (a
/// PEM block) is masked too; masking can lengthen text, so the budget is applied again after it.
pub(super) fn render(entries: &[LogEntry], format: ExportFormat, budget: usize) -> Rendered {
    let mut bytes = 0usize;
    let mut start = entries.len();
    for (ix, entry) in entries.iter().enumerate().rev() {
        let len = format.line_len(entry);
        // The newest line is always kept (cut below if it alone exceeds the budget).
        if bytes + len > budget && start < entries.len() {
            break;
        }
        bytes += len;
        start = ix;
    }
    let mut text = String::with_capacity(bytes);
    for entry in &entries[start..] {
        format.write_line(entry, &mut text);
    }
    let mut budget_cut = start > 0;
    let mut text = redact(&text).into_owned();
    if text.len() > budget {
        budget_cut = true;
        fit(&mut text, budget);
    }
    let lines = text.bytes().filter(|b| *b == b'\n').count();
    Rendered {
        text,
        lines,
        budget_cut,
    }
}

/// Drops whole lines from the front of `text` until it fits `budget`; a last line that is still
/// too long is cut on a char boundary.
fn fit(text: &mut String, budget: usize) {
    let excess = text.len() - budget;
    // The first line start at or after `excess` bytes keeps only whole lines.
    let search_from = excess.saturating_sub(1);
    let from = text.as_bytes()[search_from..]
        .iter()
        .position(|b| *b == b'\n')
        .map(|ix| search_from + ix + 1)
        .filter(|from| *from < text.len());
    match from {
        Some(from) => {
            text.drain(..from);
        }
        None => {
            let mut end = budget;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use oxikube_domain::log::LogLine;

    use super::*;
    use crate::logs::LogFilter;

    fn entry(i: usize, text: &str) -> LogEntry {
        let mut entry = LogEntry::new(LogLine::new(
            Timestamp::from_second(1_760_000_000 + i as i64).unwrap(),
            "web-0",
            "app",
            text,
        ));
        entry.seq = i as u64;
        entry
    }

    fn buffer(texts: &[&str]) -> LogBuffer {
        let mut buffer = LogBuffer::new(100);
        buffer.extend(texts.iter().enumerate().map(|(i, t)| entry(i, t)));
        buffer
    }

    #[test]
    fn pick_takes_the_newest_matches_in_order_and_counts_them_all() {
        let buffer = buffer(&["err a", "ok", "err b", "err c", "ok"]);
        let matcher = LogFilter::new("err").compile().unwrap();
        let picked = pick(&buffer, &matcher, 2);
        let texts: Vec<_> = picked.entries.iter().map(|e| &*e.text).collect();
        assert_eq!(texts, ["err b", "err c"]);
        assert_eq!((picked.scanned, picked.matched), (5, 3));
    }

    #[test]
    fn render_keeps_the_newest_lines_that_fit() {
        let entries: Vec<_> = (0..10).map(|i| entry(i, &format!("line {i:02}"))).collect();
        let format = ExportFormat::default();
        let all = render(&entries, format, 1_000);
        assert_eq!(all.lines, 10);
        assert!(!all.budget_cut);
        // Each line is 8 bytes with its newline.
        let some = render(&entries, format, 8 * 3 + 2);
        assert!(some.budget_cut);
        assert_eq!(some.text, "line 07\nline 08\nline 09\n");
    }

    #[test]
    fn a_secret_is_masked_even_when_it_spans_the_budget_check() {
        let entries = vec![entry(0, "auth Bearer abcdefghijklmnop.qrstuv ok")];
        let rendered = render(
            &entries,
            ExportFormat {
                timestamps: false,
                pod_prefix: true,
            },
            10_000,
        );
        assert!(
            !rendered.text.contains("abcdefghijklmnop"),
            "{}",
            rendered.text
        );
        assert!(rendered.text.starts_with("web-0/app auth Bearer "));
    }

    #[test]
    fn one_line_longer_than_the_budget_is_cut_on_a_char_boundary() {
        let entries = vec![entry(0, &"é".repeat(100))];
        let rendered = render(&entries, ExportFormat::default(), 11);
        assert!(rendered.budget_cut);
        assert!(rendered.text.len() <= 11);
        assert!(rendered.text.chars().all(|c| c == 'é'));
    }
}
