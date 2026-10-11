//! Selectors and the escaping of what goes into them.
//!
//! A pod, namespace or claim name reaches a query as a label value, so it is escaped here and
//! nowhere else: a value can never close its own string, add a matcher or end the selector.
//! Label *names* are fixed by the catalogue, except those of [`LabelMatch`](oxikube_domain::LabelMatch),
//! which validates them when built.

use std::fmt::{self, Write as _};

/// How a matcher compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Op {
    Eq,
    Ne,
    Re,
    Nre,
}

/// One `label op "value"` pair of a selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Matcher {
    label: String,
    op: Op,
    /// For [`Op::Re`] this is the regular expression (already built from escaped parts).
    value: String,
}

impl Matcher {
    pub(super) fn eq(label: &str, value: &str) -> Self {
        Self::new(label, Op::Eq, value)
    }

    pub(super) fn ne(label: &str, value: &str) -> Self {
        Self::new(label, Op::Ne, value)
    }

    pub(super) fn re(label: &str, regex: &str) -> Self {
        Self::new(label, Op::Re, regex)
    }

    pub(super) fn nre(label: &str, regex: &str) -> Self {
        Self::new(label, Op::Nre, regex)
    }

    /// Whether this matcher is on `label`.
    pub(super) fn is_label(&self, label: &str) -> bool {
        self.label == label
    }

    fn new(label: &str, op: Op, value: &str) -> Self {
        Self {
            label: label.to_owned(),
            op,
            value: value.to_owned(),
        }
    }

    /// `label` equals exactly one of `names` (a plain `=` for one name).
    pub(super) fn one_of(label: &str, names: &[String]) -> Self {
        match names {
            [one] => Self::eq(label, one),
            many => {
                let alternation: Vec<String> = many.iter().map(|n| regex_quote(n)).collect();
                Self::re(label, &alternation.join("|"))
            }
        }
    }
}

impl fmt::Display for Matcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = match self.op {
            Op::Eq => "=",
            Op::Ne => "!=",
            Op::Re => "=~",
            Op::Nre => "!~",
        };
        write!(f, "{}{op}{}", self.label, quote(&self.value))
    }
}

/// `metric{matcher,...}`; the braces are left out when there is no matcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Selector {
    metric: &'static str,
    matchers: Vec<Matcher>,
}

impl Selector {
    pub(super) fn new(metric: &'static str, matchers: Vec<Matcher>) -> Self {
        Self { metric, matchers }
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.metric)?;
        if self.matchers.is_empty() {
            return Ok(());
        }
        f.write_char('{')?;
        for (i, matcher) in self.matchers.iter().enumerate() {
            if i > 0 {
                f.write_char(',')?;
            }
            write!(f, "{matcher}")?;
        }
        f.write_char('}')
    }
}

/// A PromQL string literal for `value`: double-quoted, with `\`, `"`, newlines and every other
/// control character escaped.
pub(super) fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c if ('\u{80}'..='\u{9f}').contains(&c) => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `value` as a literal in an RE2 regular expression (Go's `regexp.QuoteMeta`).
pub(super) fn regex_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_quoted_and_escaped() {
        assert_eq!(quote("web-0"), r#""web-0""#);
        assert_eq!(quote(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote(r"a\b"), r#""a\\b""#);
        assert_eq!(quote("a\nb\r\tc"), r#""a\nb\r\tc""#);
        assert_eq!(quote("a\u{0}b\u{7f}\u{85}"), r#""a\x00b\x7f\u0085""#);
        assert_eq!(quote("é日本"), "\"é日本\"");
    }

    #[test]
    fn regex_metacharacters_are_literal() {
        assert_eq!(regex_quote("a.b+c"), r"a\.b\+c");
        assert_eq!(regex_quote(r"(x)|[y]{2}^$\"), r"\(x\)\|\[y\]\{2\}\^\$\\");
        assert_eq!(regex_quote("plain-name_1"), "plain-name_1");
    }

    #[test]
    fn a_regex_goes_through_both_layers() {
        let m = Matcher::one_of("pod", &["a.b".into(), r#"c"d"#.into()]);
        assert_eq!(m.to_string(), r#"pod=~"a\\.b|c\"d""#);
        let single = Matcher::one_of("pod", &["a.b".into()]);
        assert_eq!(single.to_string(), r#"pod="a.b""#);
    }

    #[test]
    fn a_selector_without_matchers_has_no_braces() {
        assert_eq!(Selector::new("up", vec![]).to_string(), "up");
        let s = Selector::new("up", vec![Matcher::eq("job", "x"), Matcher::ne("c", "")]);
        assert_eq!(s.to_string(), r#"up{job="x",c!=""}"#);
    }
}
