//! Parsing a `Warning` header value (RFC 7234 section 5.5): `warn-code SP warn-agent SP
//! warn-text [SP warn-date]`, several separated by commas.

use oxikube_ports::ApiWarning;

use crate::auth::redacted_line;

/// The longest warning text kept (a warning is one toast line; the server's are short).
const MAX_TEXT: usize = 600;

/// The warnings in one header value, in order. Malformed parts are skipped, never an error: a
/// header the server wrote must not break the request that carried it.
///
/// The text is redacted and made one bounded line before it leaves this function.
pub fn parse(value: &str) -> Vec<ApiWarning> {
    let mut out = Vec::new();
    let mut rest = value.trim_start();
    while !rest.is_empty() {
        let Some((code, after_code)) = rest.split_once(' ') else {
            break;
        };
        let Ok(code) = code.parse::<u16>() else {
            // Not a warning: resynchronise on the next comma.
            rest = skip_comma(rest);
            continue;
        };
        let after_agent = match after_code.trim_start().split_once(' ') {
            Some((_agent, tail)) => tail.trim_start(),
            None => break,
        };
        let Some((text, after_text)) = quoted(after_agent) else {
            rest = skip_comma(after_agent);
            continue;
        };
        // An optional quoted date follows the text.
        let after_date = match quoted(after_text.trim_start()) {
            Some((_date, tail)) => tail,
            None => after_text,
        };
        let text = shorten(redacted_line(&text));
        if !text.is_empty() {
            out.push(ApiWarning { code, text });
        }
        rest = skip_comma(after_date);
    }
    out
}

/// The text after the next comma (outside quotes is not tracked: callers pass a tail that
/// starts after any quoted string they understood).
fn skip_comma(s: &str) -> &str {
    match s.split_once(',') {
        Some((_, tail)) => tail.trim_start(),
        None => "",
    }
}

/// A quoted string at the start of `s` (`"..."` with `\"` and `\\` escapes): its content and
/// what follows.
fn quoted(s: &str) -> Option<(String, &str)> {
    let mut chars = s.strip_prefix('"')?.char_indices();
    let inner = &s[1..];
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                let (_, escaped) = chars.next()?;
                out.push(escaped);
            }
            '"' => return Some((out, &inner[i + 1..])),
            _ => out.push(c),
        }
    }
    None
}

fn shorten(mut text: String) -> String {
    if text.chars().count() > MAX_TEXT {
        let cut = text
            .char_indices()
            .nth(MAX_TEXT)
            .map_or(text.len(), |(i, _)| i);
        text.truncate(cut);
        text.push('…');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(value: &str) -> Vec<String> {
        parse(value).into_iter().map(|w| w.text).collect()
    }

    #[test]
    fn the_usual_kubernetes_warning() {
        let got = parse(r#"299 - "v1 ComponentStatus is deprecated in v1.19+""#);
        assert_eq!(
            got,
            vec![ApiWarning {
                code: 299,
                text: "v1 ComponentStatus is deprecated in v1.19+".into()
            }]
        );
    }

    #[test]
    fn escapes_a_date_and_several_warnings_in_one_value() {
        let got = texts(
            r#"299 agent "unknown field \"spec.foo\"" "Tue, 06 Oct 2026 10:00:00 GMT", 299 - "second one""#,
        );
        assert_eq!(got, vec![r#"unknown field "spec.foo""#, "second one"]);
    }

    #[test]
    fn malformed_values_are_skipped_not_errors() {
        assert!(parse("").is_empty());
        assert!(parse("garbage").is_empty());
        assert!(parse(r#"299 - unquoted"#).is_empty());
        assert!(parse(r#"299 - "unterminated"#).is_empty());
        assert_eq!(texts(r#"nope, 299 - "kept""#), vec!["kept"]);
    }

    #[test]
    fn the_text_is_redacted_and_bounded() {
        let long = "x ".repeat(2000);
        let got = parse(&format!(r#"299 - "{long}""#));
        assert!(got[0].text.chars().count() <= MAX_TEXT + 1);
        let secret = texts(r#"299 - "see https://user:hunter2@example.com/x""#);
        assert!(!secret[0].contains("hunter2"), "{secret:?}");
    }
}
