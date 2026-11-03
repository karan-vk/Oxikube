//! The search behind the keymap editor's input: a query is words, each of which must appear in
//! the row's title, action id, context or keystrokes.
//!
//! A word that reads as a keystroke is also tried in its canonical spelling, so `Ctrl+D`,
//! `ctrl-d` and `shift-ctrl-d` find the same rows as the keymap writes them (`ctrl-d`,
//! `ctrl-shift-d`). The haystack of each row is built once when the rows arrive
//! ([`Haystack::of`]); a keystroke of the search is a substring test over it, which keeps the
//! filter of 2 000 rows far inside the 5 ms budget.

use gpui::Keystroke;
use oxikube_keymap::{KeymapRow, RowStatus};

/// The searchable text of one row, lowercased.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Haystack(String);

impl Haystack {
    /// The text of `row`: title, action id, context (`everywhere` for a binding without one,
    /// nothing for a command bound nowhere) and keystrokes, one per line.
    pub fn of(row: &KeymapRow) -> Self {
        let mut text = String::with_capacity(
            row.title.len() + row.action.len() + row.context.as_deref().map_or(0, str::len) + 32,
        );
        for part in [
            row.title.as_str(),
            row.action.as_str(),
            match (row.status, row.context.as_deref()) {
                (RowStatus::Unbound, _) => "",
                (_, Some(context)) => context,
                (_, None) => "everywhere",
            },
        ] {
            text.push_str(part);
            text.push('\n');
        }
        text.push_str(&row.keystrokes_text());
        // The same lowercasing as the query's, so `É` finds `é`.
        Self(text.to_lowercase())
    }
}

/// A parsed search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    words: Vec<Word>,
}

/// One word: the text as typed and, when it reads as a keystroke in another spelling, that one.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Word {
    typed: String,
    canonical: Option<String>,
}

impl Query {
    /// Parse `text`. Blank text matches everything.
    pub fn parse(text: &str) -> Self {
        let words = text
            .split_whitespace()
            .map(|word| {
                let typed = word.to_lowercase();
                let canonical = canonical_keystroke(&typed).filter(|c| *c != typed);
                Word { typed, canonical }
            })
            .collect();
        Self { words }
    }

    /// Whether the query is blank.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Whether every word is found in `haystack`.
    pub fn matches(&self, haystack: &Haystack) -> bool {
        self.words.iter().all(|word| {
            haystack.0.contains(&word.typed)
                || word
                    .canonical
                    .as_ref()
                    .is_some_and(|canonical| haystack.0.contains(canonical))
        })
    }
}

/// `text` as the keymap spells a keystroke (`Ctrl+D` is `ctrl-d`, `shift-ctrl-d` is
/// `ctrl-shift-d`); `None` when it is not a keystroke or has no key (`ctrl` alone).
pub fn canonical_keystroke(text: &str) -> Option<String> {
    // `Ctrl+D` is the same as `ctrl-d`; a lone `+` or `-` is a key, not a separator.
    let spelled = if text.len() > 1 {
        text.replace('+', "-")
    } else {
        text.to_owned()
    };
    Keystroke::parse(&spelled)
        .ok()
        .filter(|stroke| !stroke.key.is_empty() && !is_modifier_name(&stroke.key))
        .map(|stroke| stroke.unparse())
}

fn is_modifier_name(key: &str) -> bool {
    matches!(
        key,
        "ctrl" | "control" | "alt" | "shift" | "cmd" | "super" | "win" | "fn" | "secondary"
    )
}
