//! The `secret-data-block` scanner: multi-line `data:` / `stringData:` blocks.
//!
//! A line ends at a real newline or at an escaped one (`\n`, `\r\n` written as backslash
//! sequences), because a manifest often arrives escaped: inside a JSON log line, or in the
//! `Debug` form of a `&str` field.

use super::scrubber::{DATA_HEADER, DATA_PAIR};
use super::values::{Cut, replace_values};
use std::borrow::Cow;

/// Redacts the entries indented under every `data:` / `stringData:` header line.
pub(super) fn scrub_data_blocks(input: &str) -> Cow<'_, str> {
    if !lines(input).any(|(content, _)| DATA_HEADER.is_match(content)) {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len());
    let mut block_indent: Option<usize> = None;
    for (content, ending) in lines(input) {
        let indent = content.len() - content.trim_start().len();
        if let Some(header_indent) = block_indent {
            if content.trim().is_empty() {
                out.push_str(content);
                out.push_str(ending);
                continue;
            }
            if indent > header_indent {
                out.push_str(&replace_values(
                    &DATA_PAIR,
                    content,
                    Cut::FramingOrKey,
                    |_| false,
                ));
                out.push_str(ending);
                continue;
            }
            block_indent = None;
        }
        if DATA_HEADER.is_match(content) {
            block_indent = Some(indent);
        }
        out.push_str(content);
        out.push_str(ending);
    }
    if out == input {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(out)
    }
}

/// Splits `input` into `(content, ending)` pairs, where `ending` is the line break that
/// followed, or `""` for the last line. A break is a real `\n` or an escaped one: `n` after a
/// run of backslashes (`\n`, or `\\n` when the text was escaped twice, as a `Debug` string
/// inside a JSON line is), and a `\r` just before it, real or escaped, belongs to it too.
/// Concatenating every pair gives `input` back.
fn lines(input: &str) -> impl Iterator<Item = (&str, &str)> {
    let mut rest = input;
    let mut done = false;
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        let bytes = rest.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            let start = match b {
                b'\n' => i,
                b'n' => match backslashes_before(bytes, i) {
                    0 => continue,
                    run => i - run,
                },
                _ => continue,
            };
            let start = carriage_return_before(bytes, start);
            let (content, tail) = rest.split_at(start);
            let (ending, next) = tail.split_at(i + 1 - start);
            rest = next;
            return Some((content, ending));
        }
        done = true;
        Some((rest, ""))
    })
}

/// Number of consecutive backslashes ending right before `end`.
fn backslashes_before(bytes: &[u8], end: usize) -> usize {
    bytes[..end]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
}

/// `start`, moved back over a real `\r` or an escaped one (`r` after backslashes).
fn carriage_return_before(bytes: &[u8], start: usize) -> usize {
    match start.checked_sub(1).map(|i| bytes[i]) {
        Some(b'\r') => start - 1,
        Some(b'r') => match backslashes_before(bytes, start - 1) {
            0 => start,
            run => start - 1 - run,
        },
        _ => start,
    }
}

#[cfg(test)]
mod tests {
    use super::lines;

    #[test]
    fn lines_split_at_real_and_escaped_breaks_and_round_trip() {
        let input = "a\nb\r\nc\\nd\\r\\ne\\\\nf\\\\r\\\\ng";
        let parts: Vec<_> = lines(input).collect();
        assert_eq!(
            parts,
            [
                ("a", "\n"),
                ("b", "\r\n"),
                ("c", "\\n"),
                ("d", "\\r\\n"),
                ("e", "\\\\n"),
                ("f", "\\\\r\\\\n"),
                ("g", "")
            ]
        );
        let joined: String = parts.iter().map(|(c, e)| format!("{c}{e}")).collect();
        assert_eq!(joined, input);
        assert_eq!(lines("").collect::<Vec<_>>(), [("", "")]);
        assert_eq!(lines("x\n").collect::<Vec<_>>(), [("x", "\n"), ("", "")]);
    }
}
