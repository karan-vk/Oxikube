//! When a string may be written as a plain YAML scalar, and how it is quoted when it may not.
//!
//! Writing a string plain is safe only when every YAML reader gives back the same string: YAML
//! 1.2 core, and the 1.1 rules the Kubernetes API server's reader still follows (`yes`, `on`,
//! `0644`, `1_000`, `12:30` and `2001-12-14` are not strings there). This is deliberately
//! conservative: a string that might read as another type is quoted, so a JSON `"yes"`, `"on"`,
//! `"123"`, `"true"` or `"null"` stays a string after the conversion.

/// Whether `text` can be written without quotes as a single-line plain scalar (a value or a key).
pub(super) fn plain_ok(text: &str) -> bool {
    let Some(first) = text.chars().next() else {
        return false;
    };
    if text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace) {
        return false;
    }
    if !text.chars().all(plain_char) {
        return false;
    }
    match first {
        '?' | ':' | '[' | ']' | '{' | '}' | ',' | '#' | '&' | '*' | '!' | '|' | '>' | '\''
        | '"' | '%' | '@' | '`' => return false,
        '-' if text.chars().nth(1).is_none_or(char::is_whitespace) => return false,
        _ => {}
    }
    if text.starts_with("---") || text.starts_with("...") {
        return false;
    }
    if text.contains(": ") || text.contains(" #") || text.ends_with(':') {
        return false;
    }
    !reads_as_another_type(text)
}

/// A character a plain scalar may hold: printable, and not a line break or tab.
fn plain_char(c: char) -> bool {
    !(c.is_control()
        || matches!(
            c,
            '\u{2028}' | '\u{2029}' | '\u{feff}' | '\u{fffe}' | '\u{ffff}'
        ))
}

/// A character a literal block scalar may hold (a tab is fine there).
pub(super) fn block_char(c: char) -> bool {
    c == '\t' || c == '\n' || plain_char(c)
}

/// `text` written as a double-quoted scalar.
pub(super) fn double_quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            '\u{85}' => out.push_str("\\N"),
            '\u{2028}' => out.push_str("\\L"),
            '\u{2029}' => out.push_str("\\P"),
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            '\u{feff}' => out.push_str("\\uFEFF"),
            '\u{fffe}' => out.push_str("\\uFFFE"),
            '\u{ffff}' => out.push_str("\\uFFFF"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether a YAML 1.1 or 1.2 reader would turn the plain scalar `text` into a null, a boolean, a
/// number or a timestamp.
fn reads_as_another_type(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "~" | "null" | "true" | "false" | "yes" | "no" | "y" | "n" | "on" | "off" | "=" | "<<"
    ) {
        return true;
    }
    is_numeric(text) || is_timestamp(text)
}

fn is_numeric(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    if body.is_empty() {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    if lower == ".inf" || lower == ".nan" {
        return true;
    }
    for (prefix, digit) in [
        (
            "0x",
            (|c: char| c.is_ascii_hexdigit() || c == '_') as fn(char) -> bool,
        ),
        ("0o", |c| matches!(c, '0'..='7' | '_')),
        ("0b", |c| matches!(c, '0' | '1' | '_')),
    ] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return !rest.is_empty() && rest.chars().all(digit);
        }
    }
    // Integers (also with 1.1 `_` and `,` separators and leading zeros) and sexagesimal `1:30`.
    if body.starts_with(|c: char| c.is_ascii_digit())
        && body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '_' | ',' | ':'))
    {
        return true;
    }
    is_decimal(body)
}

/// `digits? (. digits?)? ([eE] [-+]? digits)?` with at least one digit before the exponent;
/// sexagesimal floats (`1:30.5`) and `_` separators included.
fn is_decimal(body: &str) -> bool {
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => (&body[..at], Some(&body[at + 1..])),
        None => (body, None),
    };
    if mantissa.matches('.').count() > 1
        || !mantissa
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '_' | ':'))
        || !mantissa.chars().any(|c| c.is_ascii_digit())
    {
        return false;
    }
    match exponent {
        None => mantissa.contains('.'),
        Some(exp) => {
            let digits = exp.strip_prefix(['+', '-']).unwrap_or(exp);
            !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
        }
    }
}

/// `2001-12-14`, `2001-12-14t21:59:43.10-05:00`: YAML 1.1 timestamps.
fn is_timestamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 8
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5].is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_strings_stay_plain() {
        for text in [
            "nginx",
            "nginx:1.27",
            "registry.k8s.io/pause:3.9",
            "app.kubernetes.io/name",
            "1Gi",
            "500m",
            "5s",
            "1.27.3",
            "--port=8080",
            "-Xmx512m",
            ".dockerconfigjson",
            "a b c",
            "héllo 日本語",
            "https://example.com/a?b=c",
            "ClusterIP",
            "a#b",
            "x-1",
            "v1.2",
            "e3",
            "0.0.0.0/0",
        ] {
            assert!(plain_ok(text), "{text:?} should be plain");
        }
    }

    #[test]
    fn strings_that_read_as_another_type_are_quoted() {
        for text in [
            "",
            "~",
            "null",
            "Null",
            "NULL",
            "true",
            "False",
            "yes",
            "No",
            "on",
            "OFF",
            "y",
            "N",
            "123",
            "-5",
            "+5",
            "0644",
            "1_000",
            "1,000",
            "0x1F",
            "0o17",
            "0b101",
            "3.14",
            ".5",
            "5.",
            "1e3",
            "1E-3",
            "-.inf",
            ".NaN",
            "12:30",
            "1:30:45",
            "2001-12-14",
            "2001-12-14T21:59:43Z",
            "=",
            "<<",
        ] {
            assert!(!plain_ok(text), "{text:?} must be quoted");
        }
    }

    #[test]
    fn structure_characters_force_quotes() {
        for text in [
            " lead",
            "trail ",
            "a: b",
            "a #b",
            "trailing:",
            "- x",
            "-",
            "? x",
            ": x",
            "[a]",
            "{a}",
            "a,b", /* fine in block context but starts a flow */
            "# c",
            "&a",
            "*a",
            "!t",
            "|x",
            ">x",
            "'q",
            "\"q",
            "%d",
            "@a",
            "`a",
            "---",
            "...",
            "tab\there",
            "line\nbreak",
        ] {
            if text == "a,b" {
                assert!(plain_ok(text), "a comma inside is fine in block context");
                continue;
            }
            assert!(!plain_ok(text), "{text:?} must be quoted");
        }
    }

    #[test]
    fn double_quoting_escapes_what_it_must() {
        assert_eq!(double_quoted("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(double_quoted("l1\nl2\t\r"), "\"l1\\nl2\\t\\r\"");
        assert_eq!(double_quoted("\u{1}\u{7f}\0"), "\"\\x01\\x7f\\0\"");
        assert_eq!(
            double_quoted("\u{85}\u{2028}\u{feff}é😀"),
            "\"\\N\\L\\uFEFFé😀\""
        );
    }
}
