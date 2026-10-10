//! The tokenizer: whitespace-separated words with their spans. No quoting: a jump line is a
//! resource name and a few short words, and a pattern with a space goes in the filter bar.

use super::span::Span;

/// One word of the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Token<'a> {
    pub text: &'a str,
    pub span: Span,
}

/// Splits `line` at whitespace. A leading `:` (a pasted `:pods`) is skipped, with offsets still
/// counted from the start of `line`.
pub(super) fn tokenize(line: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut start = None;
    let skip_colon = line.trim_start().starts_with(':');
    let first_colon = line.find(':');
    for (i, c) in line.char_indices() {
        let is_colon = skip_colon && Some(i) == first_colon;
        if c.is_whitespace() || is_colon {
            if let Some(s) = start.take() {
                tokens.push(Token {
                    text: &line[s..i],
                    span: Span::new(s, i),
                });
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        tokens.push(Token {
            text: &line[s..],
            span: Span::new(s, line.len()),
        });
    }
    tokens
}

/// Whether `word` (a filter without its `/`) is a flag that takes the next word as its operand:
/// `-l selector`, `-f text`, `!-f text`.
pub(super) fn takes_operand(word: &str) -> bool {
    matches!(word, "-l" | "-f" | "!-f")
}
