//! Shaping of external free text (API `Status` messages, exec-plugin stderr, transport error
//! text) into an error-message fragment: one line, bounded length.
//!
//! These helpers do not redact. Every caller passes text that has already been through
//! [`oxikube_domain::redact::redact`], the same function the log writer and the audit path use,
//! and redacts *before* shaping so multi-line patterns (PEM blocks, YAML `data:` blocks) are
//! seen whole.

/// Longest message fragment (in characters) taken from external text.
const MAX_CHARS: usize = 300;

/// Joins the non-empty lines of `redacted` with `"; "` and bounds the length.
pub(super) fn one_line(redacted: &str) -> String {
    let lines: Vec<&str> = non_empty_lines(redacted).collect();
    truncate(&lines.join("; "))
}

/// Like [`one_line`], keeping only the last `max_lines` non-empty lines (plugin stderr puts
/// the actionable message last, after progress noise).
pub(super) fn last_lines(redacted: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = non_empty_lines(redacted).collect();
    let start = lines.len().saturating_sub(max_lines);
    truncate(&lines[start..].join("; "))
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

    #[test]
    fn prose_and_paths_pass_through() {
        let msg = "token has expired; run `aws sso login` (see /usr/local/bin/aws)";
        assert_eq!(one_line(&redact(msg)), msg);
    }
}
