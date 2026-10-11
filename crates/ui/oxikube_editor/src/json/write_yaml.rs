//! A [`Value`] as block-style YAML, in the layout of `kubectl get -o yaml`: two-space indent,
//! sequences at the indent of their key, keys in the order they have, one final newline.
//!
//! Strings are quoted whenever a YAML reader might not give back the same string (see `quote`),
//! and multi-line text is written as a literal block (`|`) when that reproduces it exactly.

use super::quote::{block_char, double_quoted, plain_ok};
use super::value::Value;

/// `value` as YAML text.
#[must_use]
pub fn to_yaml(value: &Value) -> String {
    let mut out = String::new();
    match value {
        Value::Map(entries) if !entries.is_empty() => map(&mut out, entries, 0, false),
        Value::Seq(items) if !items.is_empty() => seq(&mut out, items, 0, false),
        other => {
            inline(&mut out, other, 0);
            out.push('\n');
            if let Value::Str(text) = other
                && block_literal(text).is_some()
            {
                out.push_str(&block_body(text, 2));
            }
        }
    }
    out
}

fn pad(out: &mut String, width: usize) {
    out.extend(std::iter::repeat_n(' ', width));
}

/// The entries of a mapping whose keys sit at `indent`; the first key follows text already
/// written on its line (`- `) when `inline_first`.
fn map(out: &mut String, entries: &[(String, Value)], indent: usize, inline_first: bool) {
    for (index, (key, value)) in entries.iter().enumerate() {
        if index > 0 || !inline_first {
            pad(out, indent);
        }
        out.push_str(&scalar_text(key));
        out.push(':');
        match value {
            Value::Map(inner) if !inner.is_empty() => {
                out.push('\n');
                map(out, inner, indent + 2, false);
            }
            // kubectl layout: a sequence sits at the indent of its key.
            Value::Seq(items) if !items.is_empty() => {
                out.push('\n');
                seq(out, items, indent, false);
            }
            other => {
                out.push(' ');
                inline(out, other, indent + 2);
                out.push('\n');
            }
        }
        if let Value::Str(text) = value
            && block_literal(text).is_some()
        {
            // `inline` wrote the header line; the body follows.
            out.push_str(&block_body(text, indent + 2));
        }
    }
}

fn seq(out: &mut String, items: &[Value], indent: usize, inline_first: bool) {
    for (index, item) in items.iter().enumerate() {
        if index > 0 || !inline_first {
            pad(out, indent);
        }
        out.push_str("- ");
        match item {
            Value::Map(inner) if !inner.is_empty() => map(out, inner, indent + 2, true),
            Value::Seq(inner) if !inner.is_empty() => seq(out, inner, indent + 2, true),
            other => {
                inline(out, other, indent + 2);
                out.push('\n');
                if let Value::Str(text) = other
                    && block_literal(text).is_some()
                {
                    out.push_str(&block_body(text, indent + 2));
                }
            }
        }
    }
}

/// A value that fits after `key: ` or `- `: a scalar, `[]`, `{}`, or the `|` header of a literal
/// block (its body is written by the caller).
fn inline(out: &mut String, value: &Value, _indent: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(text) | Value::Float(text) => out.push_str(text),
        Value::Str(text) => match block_literal(text) {
            Some(header) => out.push_str(header),
            None => out.push_str(&scalar_text(text)),
        },
        Value::Seq(_) => out.push_str("[]"),
        Value::Map(_) => out.push_str("{}"),
    }
}

/// A string as a single-line scalar: plain when that is safe, double-quoted otherwise.
fn scalar_text(text: &str) -> String {
    if plain_ok(text) {
        text.to_owned()
    } else {
        double_quoted(text)
    }
}

/// The header (`|`, `|-`, `|+`) when `text` can be written as a literal block that reads back
/// exactly: several lines, only printable characters, and a first non-blank line that does not
/// start with a space (the block's indentation is taken from it).
fn block_literal(text: &str) -> Option<&'static str> {
    if !text.contains('\n') || !text.chars().all(block_char) {
        return None;
    }
    let body = text.trim_end_matches('\n');
    let first = body.split('\n').find(|line| !line.is_empty())?;
    if first.starts_with([' ', '\t']) {
        return None;
    }
    Some(match text.len() - body.len() {
        0 => "|-",
        1 => "|",
        _ => "|+",
    })
}

/// The lines of a literal block, each indented by `indent` (blank lines stay empty).
fn block_body(text: &str, indent: usize) -> String {
    let body = text.trim_end_matches('\n');
    let trailing = text.len() - body.len();
    let mut out = String::with_capacity(text.len() + body.len() / 8);
    for line in body.split('\n') {
        if !line.is_empty() {
            pad(&mut out, indent);
            out.push_str(line);
        }
        out.push('\n');
    }
    // `|+` keeps the blank lines after the last one.
    for _ in 1..trailing {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests;
