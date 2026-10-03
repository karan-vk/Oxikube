//! Message scrubbing for text that originates outside Oxikube (API `Status` messages,
//! exec-plugin stderr, transport error text).
//!
//! This is a deliberately small stand-in: E03-S08 lands `oxikube_domain::redact`, and
//! this module is to be deleted in favour of it. Until then it errs on the side of
//! over-redacting. Messages built by this crate never contain credential material
//! by construction (no exec command line, no plugin stdout); this is the second line
//! of defence for the free text that is included.

/// Placeholder written in place of a redacted value.
pub(crate) const REDACTED: &str = "[REDACTED]";

/// Longest message fragment (in characters) taken from external text.
const MAX_CHARS: usize = 300;

/// Key names whose value must never be shown (matched on a lowercased suffix).
const SENSITIVE_KEYS: &[&str] = &[
    "token",
    "password",
    "passwd",
    "secret",
    "client-key-data",
    "client_key_data",
    "apikey",
    "api_key",
    "authorization",
];

/// Scrubs `text` of credential-shaped material and bounds its length.
///
/// Multi-line input is joined with `"; "` (empty lines dropped) so the result is a
/// single line suitable for an error message.
pub(crate) fn scrub(text: &str) -> String {
    let joined = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(scrub_line)
        .collect::<Vec<_>>()
        .join("; ");
    truncate(&joined)
}

/// Scrubs `text`, keeping only its last `max_lines` non-empty lines (plugin stderr puts
/// the actionable message last, after progress noise).
pub(crate) fn scrub_tail(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    scrub(&lines[start..].join("\n"))
}

fn truncate(s: &str) -> String {
    if s.chars().count() <= MAX_CHARS {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(MAX_CHARS).collect();
    out.push('…');
    out
}

fn scrub_line(line: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut redact_next = false;
    for tok in line.split_whitespace() {
        let bare = tok.trim_matches(is_wrapper).to_ascii_lowercase();
        let is_scheme = bare == "bearer" || bare == "basic";
        if redact_next && !is_scheme {
            redact_next = false;
            out.push(REDACTED.to_owned());
        } else if is_scheme {
            // `Authorization: Bearer <tok>`: keep the scheme word, hide what follows it.
            redact_next = true;
            out.push(tok.to_owned());
        } else if let Some((key, value)) = split_key_value(tok) {
            if value.trim_matches(is_wrapper).is_empty() {
                redact_next = true;
                out.push(tok.to_owned());
            } else {
                out.push(format!("{key}{REDACTED}"));
            }
        } else if looks_like_secret(tok.trim_matches(is_wrapper)) {
            out.push(REDACTED.to_owned());
        } else {
            out.push(tok.to_owned());
        }
    }
    out.join(" ")
}

fn is_wrapper(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | ',' | ';' | '(' | ')' | '{' | '}' | '[' | ']' | '`'
    )
}

/// Splits `key=value` / `key:value` / `"key":"value"` when the key is sensitive.
/// Returns the text up to and including the separator, and the value.
fn split_key_value(tok: &str) -> Option<(&str, &str)> {
    let idx = tok.find([':', '='])?;
    let (key, rest) = tok.split_at(idx + 1);
    let name = key
        .trim_end_matches([':', '='])
        .trim_matches(is_wrapper)
        .to_ascii_lowercase();
    SENSITIVE_KEYS
        .iter()
        .any(|k| name.ends_with(k))
        .then_some((key, rest))
}

/// JWT-shaped, or a long unbroken base64/hex-like run (bearer tokens, key material).
fn looks_like_secret(tok: &str) -> bool {
    if tok.len() >= 20 && tok.starts_with("eyJ") {
        return true;
    }
    tok.len() >= 40
        && tok
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '=' | '.'))
        && tok.chars().any(|c| c.is_ascii_digit())
        && tok.chars().any(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_bearer_and_basic_credentials() {
        let out = scrub("Authorization: Bearer FAKE-abc123 rejected");
        assert!(!out.contains("FAKE-abc123"), "{out}");
        assert!(out.contains("Bearer"));
        assert!(!scrub("Basic dXNlcjpwYXNz failed").contains("dXNlcjpwYXNz"));
    }

    #[test]
    fn redacts_key_value_forms() {
        for s in [
            "token=FAKEVALUE",
            r#"{"access_token":"FAKEVALUE"}"#,
            "password: FAKEVALUE",
            "client-key-data=FAKEVALUE",
        ] {
            let out = scrub(s);
            assert!(!out.contains("FAKEVALUE"), "{s} -> {out}");
        }
    }

    #[test]
    fn redacts_jwt_and_long_opaque_runs() {
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJmYWtlIn0.c2ln";
        assert!(!scrub(&format!("got {jwt} from plugin")).contains("eyJ"));
        let opaque = "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8S9t0U1v2";
        assert!(!scrub(opaque).contains(opaque));
    }

    #[test]
    fn leaves_prose_and_paths_alone() {
        let msg = "token has expired; run `aws sso login` (see /usr/local/bin/aws)";
        assert_eq!(scrub(msg), msg);
    }

    #[test]
    fn bounds_length_and_joins_lines() {
        let long = "x ".repeat(400);
        assert!(scrub(&long).chars().count() <= MAX_CHARS + 1);
        assert_eq!(scrub("one\n\n two \n"), "one; two");
    }

    #[test]
    fn tail_keeps_last_lines() {
        assert_eq!(scrub_tail("a\nb\nc\nd", 2), "c; d");
    }
}
