//! Keeping credentials out of the describe text.
//!
//! deskribe prints a Secret's `Data` as `key:  N bytes`, with one deliberate exception copied
//! from `kubectl`: the decoded `token` of a `kubernetes.io/service-account-token` Secret. That is
//! a live bearer credential, and the Describe tab can be copied from, so it is replaced by
//! [`HIDDEN`] here, the same word the YAML tab uses.

/// What stands in for a value that is not shown (the YAML tab says the same).
pub(crate) const HIDDEN: &str = "(hidden)";

/// `text` with every value in a Secret's `Data` section that is not a byte count replaced by
/// [`HIDDEN`]. Lines outside that section are untouched; a value that spans lines is collapsed
/// into the one `key:  (hidden)` line.
pub(crate) fn mask_secret_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_data = false;
    let mut after_key = false;
    let mut lines = text.split_inclusive('\n').peekable();
    while let Some(raw) = lines.next() {
        let line = raw.trim_end_matches(['\n', '\r']);
        let ending = &raw[line.len()..];
        if !in_data {
            in_data = line.trim() == "Data" && lines.peek().is_some_and(|n| n.trim() == "====");
            out.push_str(raw);
            continue;
        }
        if line.trim().is_empty() {
            after_key = false;
            out.push_str(raw);
        } else if line.trim() == "====" || is_byte_count(line) {
            after_key = true;
            out.push_str(raw);
        } else if let Some(prefix) = key_prefix(line) {
            out.push_str(prefix);
            out.push_str(HIDDEN);
            out.push_str(ending);
            after_key = true;
        } else if !after_key {
            // A line of no known shape in the section: say nothing about it.
            out.push_str(HIDDEN);
            out.push_str(ending);
        }
        // Otherwise a continuation of a value already replaced: dropped.
    }
    out
}

/// `key:` followed by a decoded byte count, as deskribe writes every ordinary Secret value.
fn is_byte_count(line: &str) -> bool {
    let Some((key, value)) = line.split_once(':') else {
        return false;
    };
    !key.is_empty()
        && !key.contains(char::is_whitespace)
        && value
            .trim()
            .strip_suffix(" bytes")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The `key:` and the padding after it, when `line` starts a `key: value` entry.
fn key_prefix(line: &str) -> Option<&str> {
    let colon = line.find(':')?;
    let key = &line[..colon];
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    let rest = &line[colon + 1..];
    let padding = rest.len() - rest.trim_start().len();
    Some(&line[..colon + 1 + padding])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_value_is_replaced_and_byte_counts_are_kept() {
        let text = "Name:  sa-token\nType:  kubernetes.io/service-account-token\n\nData\n====\nca.crt:     1066 bytes\ntoken:      eyJhbGciOi.payload.sig\nnamespace:  7 bytes\n";
        let masked = mask_secret_text(text);
        assert!(!masked.contains("eyJhbGciOi"), "{masked}");
        assert!(masked.contains("token:      (hidden)\n"), "{masked}");
        assert!(masked.contains("ca.crt:     1066 bytes\n"), "{masked}");
        assert!(masked.contains("namespace:  7 bytes\n"), "{masked}");
        assert!(masked.starts_with("Name:  sa-token\n"), "{masked}");
    }

    #[test]
    fn a_value_over_several_lines_leaves_no_trace() {
        let text = "Data\n====\ntoken:  line-one\nline-two more\nca.crt:  5 bytes\n";
        let masked = mask_secret_text(text);
        assert!(!masked.contains("line-"), "{masked}");
        assert!(!masked.contains("more"), "{masked}");
        assert!(masked.contains("ca.crt:  5 bytes"), "{masked}");
    }

    #[test]
    fn text_without_a_data_section_is_unchanged() {
        let text = "Name:  x\nLabels:  a=b\n";
        assert_eq!(mask_secret_text(text), text);
    }
}
