//! [`Language`]: whether a buffer is read as YAML or as JSON, and how that is decided.
//!
//! The language is editor state, never buffer text: switching it changes the grammar, the checks
//! and the convert command on offer, and touches neither the text nor the undo history.

use std::fmt;

/// What a manifest buffer is written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Language {
    /// YAML 1.2 (the default; also what `kubectl get -o yaml` prints).
    #[default]
    Yaml,
    /// JSON (`kubectl get -o json`, API payloads, CRD examples).
    Json,
}

impl Language {
    /// The tree-sitter grammar name of this language (`"yaml"`, `"json"`); also the `mode`
    /// value of the editor's key context.
    #[must_use]
    pub const fn grammar(self) -> &'static str {
        match self {
            Language::Yaml => "yaml",
            Language::Json => "json",
        }
    }

    /// How the editor names the language to the user (`"YAML"`, `"JSON"`).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Language::Yaml => "YAML",
            Language::Json => "JSON",
        }
    }

    /// The other language: what a conversion produces.
    #[must_use]
    pub const fn other(self) -> Language {
        match self {
            Language::Yaml => Language::Json,
            Language::Json => Language::Yaml,
        }
    }

    /// Content sniffing: JSON when the first non-blank character is `{` or `[`, YAML otherwise
    /// (a YAML mapping, a `---` marker, a comment, an empty buffer). A byte-order mark is
    /// skipped. Costs the length of the leading blank run, not of the buffer.
    #[must_use]
    pub fn sniff(text: &str) -> Language {
        let first = text
            .trim_start_matches(|c: char| c == '\u{feff}' || c.is_whitespace())
            .chars()
            .next();
        match first {
            Some('{' | '[') => Language::Json,
            _ => Language::Yaml,
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// How an editor chooses its [`Language`]: from the content as it changes, or fixed by the user
/// (the command, the toolbar chip), by the tab it was opened as, or by a conversion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LanguageMode {
    /// Follow the content: [`Language::sniff`] on every check.
    #[default]
    Auto,
    /// Always this language, whatever the content looks like.
    Fixed(Language),
}

impl LanguageMode {
    /// The language of `text` under this mode.
    #[must_use]
    pub fn resolve(self, text: &str) -> Language {
        match self {
            LanguageMode::Auto => Language::sniff(text),
            LanguageMode::Fixed(language) => language,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_brace_or_bracket_means_json() {
        assert_eq!(Language::sniff("{\"kind\": \"Pod\"}"), Language::Json);
        assert_eq!(Language::sniff("  \n\t[ {\"a\": 1} ]"), Language::Json);
        assert_eq!(Language::sniff("\u{feff}{}"), Language::Json);
    }

    #[test]
    fn yaml_and_empty_text_are_yaml() {
        assert_eq!(Language::sniff("kind: Pod\n"), Language::Yaml);
        assert_eq!(Language::sniff("---\nkind: Pod"), Language::Yaml);
        assert_eq!(Language::sniff("# {not json}\nkind: Pod"), Language::Yaml);
        assert_eq!(Language::sniff("- {a: 1}"), Language::Yaml);
        assert_eq!(Language::sniff(""), Language::Yaml);
        assert_eq!(Language::sniff("   \n"), Language::Yaml);
    }

    #[test]
    fn a_fixed_mode_beats_the_content() {
        let json = "{\"a\": 1}";
        assert_eq!(LanguageMode::Auto.resolve(json), Language::Json);
        assert_eq!(
            LanguageMode::Fixed(Language::Yaml).resolve(json),
            Language::Yaml
        );
        assert_eq!(
            LanguageMode::Fixed(Language::Json).resolve("a: 1"),
            Language::Json
        );
    }
}
