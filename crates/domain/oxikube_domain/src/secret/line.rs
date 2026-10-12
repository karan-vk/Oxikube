//! [`DecodedValue::summary_line`]: a value cut to one line for a list row.

use super::decoded::{DecodedValue, placeholder};

impl DecodedValue {
    /// The first line of the value, cut to `max_chars` (with `…`); a multi-line value says how many
    /// more lines it has (`  (+2 lines)`); a binary value is its placeholder. Walks the text once,
    /// so make it when the value is read, not on every frame.
    pub fn summary_line(&self, max_chars: usize) -> String {
        let text = match self {
            DecodedValue::Binary { len, .. } => return placeholder(*len),
            DecodedValue::Text(text) => text.expose(),
        };
        let first = text.lines().next().unwrap_or_default();
        let mut shown: String = first.chars().take(max_chars).collect();
        if first.chars().nth(max_chars).is_some() {
            shown.push('…');
        }
        let more = text.lines().count().saturating_sub(1);
        if more > 0 {
            shown.push_str(&format!("  (+{more} lines)"));
        }
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::SecretText;

    fn text(value: &str) -> DecodedValue {
        DecodedValue::Text(SecretText::new(value))
    }

    #[test]
    fn a_short_value_is_shown_whole() {
        assert_eq!(text("hunter2").summary_line(96), "hunter2");
        assert_eq!(text("").summary_line(96), "");
    }

    #[test]
    fn a_long_first_line_is_cut_and_marked() {
        assert_eq!(text(&"x".repeat(10)).summary_line(10), "x".repeat(10));
        assert_eq!(
            text(&"x".repeat(11)).summary_line(10),
            format!("{}…", "x".repeat(10))
        );
    }

    #[test]
    fn extra_lines_are_counted() {
        assert_eq!(text("a\nb\nc").summary_line(96), "a  (+2 lines)");
    }

    #[test]
    fn a_binary_value_is_its_size() {
        let binary = DecodedValue::from_base64("//4=");
        assert_eq!(binary.summary_line(96), "(binary, 2 bytes)");
    }
}
