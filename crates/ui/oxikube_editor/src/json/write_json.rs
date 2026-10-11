//! A [`Value`] as pretty JSON: two-space indent, keys in the order they have, one final newline.

use std::fmt::Write as _;

use super::value::Value;

/// `value` as JSON text. Numbers are written with the digits they hold.
#[must_use]
pub fn to_json(value: &Value) -> String {
    let mut out = String::new();
    write(&mut out, value, 0);
    out.push('\n');
    out
}

fn write(out: &mut String, value: &Value, depth: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(text) | Value::Float(text) => out.push_str(text),
        Value::Str(text) => string(out, text),
        Value::Seq(items) if items.is_empty() => out.push_str("[]"),
        Value::Seq(items) => {
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                indent(out, depth + 1);
                write(out, item, depth + 1);
                out.push_str(if index + 1 < items.len() { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push(']');
        }
        Value::Map(entries) if entries.is_empty() => out.push_str("{}"),
        Value::Map(entries) => {
            out.push_str("{\n");
            for (index, (key, item)) in entries.iter().enumerate() {
                indent(out, depth + 1);
                string(out, key);
                out.push_str(": ");
                write(out, item, depth + 1);
                out.push_str(if index + 1 < entries.len() {
                    ",\n"
                } else {
                    "\n"
                });
            }
            indent(out, depth);
            out.push('}');
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// A JSON string literal: `"`, `\`, control characters and the few characters YAML readers
/// treat specially (U+2028, U+2029, U+FEFF) are escaped; other non-ASCII text, `<`, `>` and `&` stay
/// as they are.
fn string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            // Characters some readers take for line breaks or drop, and the controls: escaped.
            c if c.is_control()
                || matches!(
                    c,
                    '\u{2028}' | '\u{2029}' | '\u{feff}' | '\u{fffe}' | '\u{ffff}'
                ) =>
            {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_pretty_json_in_key_order() {
        let value = Value::Map(vec![
            ("b".into(), Value::Int("1".into())),
            (
                "a".into(),
                Value::Seq(vec![
                    Value::Bool(true),
                    Value::Null,
                    Value::Float("0.5".into()),
                ]),
            ),
            ("empty".into(), Value::Map(vec![])),
            ("none".into(), Value::Seq(vec![])),
        ]);
        assert_eq!(
            to_json(&value),
            "{\n  \"b\": 1,\n  \"a\": [\n    true,\n    null,\n    0.5\n  ],\n  \"empty\": {},\n  \"none\": []\n}\n"
        );
    }

    #[test]
    fn escapes_what_json_requires_and_nothing_more() {
        let value = Value::Str("q\"b\\n\n\t\u{1}é<>&😀".into());
        assert_eq!(to_json(&value), "\"q\\\"b\\\\n\\n\\t\\u0001é<>&😀\"\n");
    }
}
