//! Value replacement for the `key: value` patterns: keep the key and separator, replace the
//! value with [`MARKER`] in the value's own quote style, and cut a bare word where it ends.

use super::patterns::MARKER;
use regex::{Captures, Regex};
use std::borrow::Cow;

/// Where a bare (unquoted) value may end early, besides at whitespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cut {
    /// At a `, ; ) } ]` followed by the end of the word, another of those, or a quote.
    Framing,
    /// As [`Cut::Framing`], and also before another `key:` / `key=` (compact data maps such as
    /// `{a: b,c: d}`, where the next entry must be scanned on its own).
    FramingOrKey,
}

/// Replaces the `val` group of every match of `re` in `input`, keeping the text before it.
///
/// A `bare` group that ends the value is cut by [`bare_len`]; scanning resumes right after the
/// (possibly cut) value, so whatever was cut off is scanned again. When `skip` returns true for
/// a match, it is left alone and scanning resumes at the start of its value, so a key whose
/// value another stage handles (a data map) cannot hide a nested secret.
pub(super) fn replace_values<'a>(
    re: &Regex,
    input: &'a str,
    cut: Cut,
    skip: impl Fn(&Captures<'_>) -> bool,
) -> Cow<'a, str> {
    let mut out = String::new();
    let mut copied = 0;
    let mut pos = 0;
    while pos <= input.len() {
        let Some(caps) = re.captures_at(input, pos) else {
            break;
        };
        let Some(val) = caps.name("val") else {
            break;
        };
        if skip(&caps) {
            pos = val.start();
            continue;
        }
        let (start, end) = match caps.name("bare") {
            Some(bare) if bare.range() == val.range() => {
                // An escaped line break right after the separator (`stringData:\n  k: v` in a
                // JSON string) is not part of the value: it ends the key's line.
                let start = bare.start() + escaped_break_len(bare.as_str());
                let after_quotes = input[start..].trim_start_matches(['"', '\'', '\\']);
                if start > bare.start() && after_quotes.starts_with(MARKER) {
                    // Already redacted on an earlier pass (`key:\n"[redacted]"`).
                    pos = input.len() - after_quotes.len() + MARKER.len();
                    continue;
                }
                // `bare_len` skips escape pairs, so it never cuts inside the break.
                let end = bare.start() + bare_len(bare.as_str(), cut);
                (start, end.max(start))
            }
            _ => (val.start(), val.end()),
        };
        if end == start {
            // The bare word was all framing (`token=, next`) or a line break: no value here.
            pos = start;
            continue;
        }
        if let Some(rep) = replacement(&input[start..end]) {
            out.push_str(&input[copied..start]);
            out.push_str(rep);
            copied = end;
        }
        pos = end;
    }
    if copied == 0 {
        return Cow::Borrowed(input);
    }
    out.push_str(&input[copied..]);
    Cow::Owned(out)
}

/// The replacement for `val`, keeping its quote style, or `None` when it is already redacted.
pub(super) fn replacement(val: &str) -> Option<&'static str> {
    if val.trim_matches(['"', '\'', '\\']) == MARKER {
        None
    } else if val.starts_with("\\\"") {
        Some("\\\"[redacted]\\\"")
    } else if val.starts_with('"') {
        Some("\"[redacted]\"")
    } else if val.starts_with('\'') {
        Some("'[redacted]'")
    } else {
        Some(MARKER)
    }
}

/// Length of the escaped line breaks (`\n`, `\r\n`, `\\n` when escaped twice) that open
/// `word`.
fn escaped_break_len(word: &str) -> usize {
    let bytes = word.as_bytes();
    let mut len = 0;
    loop {
        let run = bytes[len..].iter().take_while(|&&b| b == b'\\').count();
        match bytes.get(len + run) {
            Some(b'n' | b'r') if run > 0 => len += run + 1,
            _ => return len,
        }
    }
}

/// Length of the bare word `word` once cut at the first `, ; ) } ]` that ends the value.
/// Escape pairs (`\,`) are part of the value.
fn bare_len(word: &str, cut: Cut) -> usize {
    let bytes = word.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            // Every byte compared here is ASCII, so skipping into a multi-byte character is
            // harmless: continuation bytes never match.
            b'\\' => i += 2,
            b',' | b';' | b')' | b'}' | b']' if ends_value(&bytes[i + 1..], cut) => return i,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Whether a closer followed by `next` (the rest of the bare word) ends the value.
fn ends_value(next: &[u8], cut: Cut) -> bool {
    match next.first() {
        None | Some(b',' | b';' | b')' | b'}' | b']' | b'"' | b'\'') => true,
        Some(b'\\') => matches!(next.get(1), Some(b'"' | b'\'')),
        Some(_) => cut == Cut::FramingOrKey && starts_with_key(next),
    }
}

/// `next` opens with `name:` or `name=` (name of `[A-Za-z0-9_.-]`).
fn starts_with_key(next: &[u8]) -> bool {
    let name = next
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
        .count();
    name > 0
        && next[name..]
            .iter()
            .find(|b| !matches!(b, b' ' | b'\t'))
            .is_some_and(|b| matches!(b, b':' | b'='))
}
