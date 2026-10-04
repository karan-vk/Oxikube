//! Shaping of external free text (API `Status` messages, exec-plugin stderr, transport error
//! text) into an error-message fragment: one line, bounded length, long opaque runs masked.
//!
//! Every caller passes text that has already been through [`oxikube_domain::redact::redact`],
//! the same function the log writer and the audit path use, and redacts *before* shaping so
//! multi-line patterns (PEM blocks, YAML `data:` blocks) are seen whole. On top of that, the
//! shaping masks long unbroken base64/hex-like runs ([`mask_opaque_runs`]), as the S04 stand-in
//! did: `redact` deliberately leaves shape-only matches alone (a UID or digest in a log line is
//! useful), but in an error message shown to the user an unlabelled 40-character token is
//! more likely a credential echoed by a plugin than anything worth reading.

use oxikube_domain::redact::MARKER;

/// Longest message fragment (in characters) taken from external text.
const MAX_CHARS: usize = 300;

/// Shortest run [`mask_opaque_runs`] treats as an opaque credential.
const OPAQUE_MIN_LEN: usize = 40;

/// Joins the non-empty lines of `redacted` with `"; "`, masks opaque runs and bounds the length.
pub(super) fn one_line(redacted: &str) -> String {
    let lines: Vec<&str> = non_empty_lines(redacted).collect();
    truncate(&mask_opaque_runs(&lines.join("; ")))
}

/// Like [`one_line`], keeping only the last `max_lines` non-empty lines (plugin stderr puts
/// the actionable message last, after progress noise).
pub(super) fn last_lines(redacted: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = non_empty_lines(redacted).collect();
    let start = lines.len().saturating_sub(max_lines);
    truncate(&mask_opaque_runs(&lines[start..].join("; ")))
}

/// Replaces every space-separated word that is a long opaque run (see [`is_opaque`]), ignoring
/// wrapping quotes and brackets, with [`MARKER`].
fn mask_opaque_runs(line: &str) -> String {
    line.split(' ')
        .map(|word| {
            let bare = word.trim_matches(is_wrapper);
            if is_opaque(bare) {
                word.replacen(bare, MARKER, 1)
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_wrapper(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | ',' | ';' | ':' | '.' | '(' | ')' | '{' | '}' | '[' | ']' | '`'
    )
}

/// At least [`OPAQUE_MIN_LEN`] base64url/hex-like characters, mixing letters and digits (so a
/// long English word is not one; `/` and `:` are excluded so paths and URLs are not either).
fn is_opaque(word: &str) -> bool {
    word.len() >= OPAQUE_MIN_LEN
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '=' | '.'))
        && word.chars().any(|c| c.is_ascii_digit())
        && word.chars().any(|c| c.is_ascii_alphabetic())
}

fn non_empty_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty())
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
    use oxikube_domain::redact::redact;

    use super::*;

    #[test]
    fn joins_lines_and_bounds_length() {
        let long = "x ".repeat(400);
        assert!(one_line(&long).chars().count() <= MAX_CHARS + 1);
        assert_eq!(one_line("one\n\n two \n"), "one; two");
    }

    #[test]
    fn last_lines_keeps_the_tail() {
        assert_eq!(last_lines("a\nb\nc\nd", 2), "c; d");
        assert_eq!(last_lines("", 3), "");
    }

    #[test]
    fn redacting_before_shaping_catches_multi_line_secrets() {
        let pem =
            "before\n-----BEGIN PRIVATE KEY-----\nZmFrZS1rZXk=\n-----END PRIVATE KEY-----\nafter";
        let out = one_line(&redact(pem));
        assert!(!out.contains("ZmFrZS1rZXk="), "{out}");
        assert!(
            out.starts_with("before; ") && out.ends_with("; after"),
            "{out}"
        );
        let stderr = "progress\nAuthorization: Bearer FAKE-abc123xyz rejected";
        let out = last_lines(&redact(stderr), 1);
        assert!(!out.contains("FAKE-abc123xyz"), "{out}");
        assert!(out.contains("rejected"), "{out}");
    }

    /// The cases of the deleted S04 `auth::scrub` tests, against the replacement pipeline.
    #[test]
    fn s04_scrub_cases_stay_redacted() {
        let out = one_line(&redact("Authorization: Bearer FAKE-abc123 rejected"));
        assert_eq!(out, "Authorization: [redacted] rejected");
        let out = one_line(&redact("retry with Bearer FAKE-abc123"));
        assert_eq!(out, "retry with Bearer [redacted]");
        for (input, secret) in [
            ("Basic dXNlcjpwYXNz failed", "dXNlcjpwYXNz"),
            (
                "error: Basic dXNlcjpwYXNzd29yZA== rejected",
                "dXNlcjpwYXNzd29yZA==",
            ),
            ("token=FAKEVALUE", "FAKEVALUE"),
            (r#"{"access_token":"FAKEVALUE"}"#, "FAKEVALUE"),
            ("password: FAKEVALUE", "FAKEVALUE"),
            ("client-key-data=FAKEVALUE", "FAKEVALUE"),
            ("error: api_key=FAKEVALUE", "FAKEVALUE"),
            ("x-api-key: FAKEVALUE", "FAKEVALUE"),
            (r#"{"apikey":"FAKEVALUE"}"#, "FAKEVALUE"),
            ("secret: FAKEVALUE", "FAKEVALUE"),
            ("Bearer SHORT1 rejected", "SHORT1"),
        ] {
            let out = one_line(&redact(input));
            assert!(!out.contains(secret), "{input} -> {out}");
            let out = last_lines(&redact(&format!("noise\n{input}")), 1);
            assert!(!out.contains(secret), "{input} -> {out}");
        }
    }

    #[test]
    fn jwt_and_long_opaque_runs_are_masked() {
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJmYWtlIn0.c2ln";
        let out = one_line(&redact(&format!("got {jwt} from plugin")));
        assert!(!out.contains("eyJ"), "{out}");
        let opaque = "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8S9t0U1v2";
        let out = one_line(&redact(&format!("got {opaque} from plugin")));
        assert_eq!(out, "got [redacted] from plugin");
        let out = last_lines(&format!("progress\nfailed: \"{opaque}\"."), 1);
        assert_eq!(out, "failed: \"[redacted]\".");
    }

    #[test]
    fn short_ids_long_words_and_urls_are_not_opaque() {
        for text in [
            "pod 3f2b9c1e-8a47-4d52-9e0b-6c1d2a7f5e84 not found",
            "unrecognizedauthenticationmechanismnameprovided here",
            "see https://login.microsoftonline.com/0123456789abcdef/oauth2/v2.0/token",
            "read /var/run/secrets/kubernetes.io/serviceaccount/token0123456789",
        ] {
            assert_eq!(one_line(text), text);
        }
    }

    #[test]
    fn prose_and_paths_pass_through() {
        let msg = "token has expired; run `aws sso login` (see /usr/local/bin/aws)";
        assert_eq!(one_line(&redact(msg)), msg);
    }
}
