//! [`Warnings`]: what a conversion could not carry over, counted for the one-line warning.

/// What a conversion lost or read differently from the plain text. All zero means a faithful
/// conversion; [`summary`](Self::summary) is the line shown to the user otherwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Warnings {
    /// YAML comments dropped (JSON has none). Counts every `#` comment of the source.
    pub comments: usize,
    /// Aliases (`*name`) replaced by a copy of the anchored value; the sharing is gone.
    pub aliases: usize,
    /// Tags (`!!binary`, `!custom`) dropped; the value is kept untagged.
    pub tags: usize,
    /// Integers written with a leading zero (`0644`) read as octal, like the API server does.
    pub octal: usize,
    /// Plain `yes`, `no`, `on`, `off`, `y`, `n` kept as strings (YAML 1.2). The API server's
    /// YAML reader takes them for booleans; JSON states the type, so it is now explicit.
    pub bool_words: usize,
    /// Documents of a multi-document YAML wrapped into one `kind: List`; 0 for a single one.
    pub documents: usize,
}

impl Warnings {
    /// Nothing was lost or reinterpreted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Warnings::default()
    }

    /// Whether the conversion dropped information (comments, tags, sharing) rather than only
    /// stating something the source left to the reader.
    #[must_use]
    pub fn lossy(&self) -> bool {
        self.comments + self.aliases + self.tags > 0
    }

    /// The warning as one line without a final stop, `None` when there is nothing to say:
    /// "3 comments dropped · 2 aliases expanded".
    #[must_use]
    pub fn summary(&self) -> Option<String> {
        let part = |n: usize, one: &str, many: &str, rest: &str| {
            (n > 0).then(|| format!("{n} {} {rest}", if n == 1 { one } else { many }))
        };
        let parts: Vec<String> = [
            part(self.comments, "comment", "comments", "dropped"),
            part(self.aliases, "alias", "aliases", "expanded"),
            part(self.tags, "tag", "tags", "dropped"),
            part(self.octal, "integer", "integers", "read as octal"),
            part(
                self.bool_words,
                "yes/no word",
                "yes/no words",
                "kept as strings",
            ),
            part(self.documents, "document", "documents", "wrapped in a List"),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_faithful_conversion_says_nothing() {
        assert_eq!(Warnings::default().summary(), None);
        assert!(Warnings::default().is_empty());
        assert!(!Warnings::default().lossy());
    }

    #[test]
    fn the_summary_counts_and_pluralises() {
        let warnings = Warnings {
            comments: 3,
            aliases: 1,
            documents: 2,
            ..Warnings::default()
        };
        assert_eq!(
            warnings.summary().as_deref(),
            Some("3 comments dropped · 1 alias expanded · 2 documents wrapped in a List")
        );
        assert!(warnings.lossy());
        let one = Warnings {
            comments: 1,
            ..Warnings::default()
        };
        assert_eq!(one.summary().as_deref(), Some("1 comment dropped"));
    }

    #[test]
    fn readings_alone_are_not_a_loss() {
        let octal = Warnings {
            octal: 1,
            ..Warnings::default()
        };
        assert!(!octal.lossy());
        assert!(!octal.is_empty());
    }
}
