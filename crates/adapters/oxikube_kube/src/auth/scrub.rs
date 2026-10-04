//! Message shaping for text that originates outside Oxikube (API `Status` messages,
//! exec-plugin stderr, transport error text).
//!
//! Redaction itself is `oxikube_domain::redact`, the same function the log writer and the
//! audit path use; this module only makes the text fit an error message: one line, bounded
//! length. Messages built by this crate never contain credential material by construction
//! (no exec command line, no plugin stdout); redaction is the second line of defence for the
//! free text that is included.

use oxikube_domain::redact::redact;

/// Longest message fragment (in characters) taken from external text.
const MAX_CHARS: usize = 300;

/// Redacts `text` and bounds its length.
///
/// Multi-line input is joined with `"; "` (empty lines dropped) so the result is a
/// single line suitable for an error message. Redaction runs first, on the original
/// text, so multi-line patterns (PEM blocks, YAML `data:` blocks) are seen whole.
pub(crate) fn scrub(text: &str) -> String {
    let redacted = redact(text);
    let joined = redacted
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    truncate(&joined)
}

/// Scrubs `text`, keeping only its last `max_lines` non-empty lines (plugin stderr puts
/// the actionable message last, after progress noise).
pub(crate) fn scrub_tail(text: &str, max_lines: usize) -> String {
    let redacted = redact(text);
    let lines: Vec<&str> = redacted
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    truncate(&lines[start..].join("; "))
}

fn truncate(s: &str) -> String {
    if s.chars().count() <= MAX_CHARS {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(MAX_CHARS).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_authorization_credentials() {
        let out = scrub("Authorization: Bearer FAKE-abc123xyz rejected");
        assert!(!out.contains("FAKE-abc123xyz"), "{out}");
        assert!(out.contains("rejected"), "{out}");
        assert!(!scrub("Authorization: Basic dXNlcjpwYXNz failed").contains("dXNlcjpwYXNz"));
        assert!(!scrub("sent Bearer FAKE-abc123xyz upstream").contains("FAKE-abc123xyz"));
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
    fn redacts_jwt_and_pem_blocks_across_lines() {
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJmYWtlIn0.c2lnbmF0dXJl";
        assert!(!scrub(&format!("got {jwt} from plugin")).contains("eyJ"));
        let pem =
            "before\n-----BEGIN PRIVATE KEY-----\nZmFrZS1rZXk=\n-----END PRIVATE KEY-----\nafter";
        let out = scrub(pem);
        assert!(!out.contains("ZmFrZS1rZXk="), "{out}");
        assert!(
            out.starts_with("before; ") && out.ends_with("; after"),
            "{out}"
        );
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
