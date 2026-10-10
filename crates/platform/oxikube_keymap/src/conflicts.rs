//! Duplicate bindings: the same keystrokes in the same context given two different meanings in
//! one layer.
//!
//! The later one wins (GPUI takes the binding added last), so nothing breaks, but the earlier one
//! is dead and the author probably did not mean it. The report is data for the help overlay
//! (E11-S10) and the future keymap editor (E21); it is also logged when the user's file has any.
//! A user binding that replaces a *default* is not a conflict, it is an override, and the
//! binding's layer ([`crate::KeybindSource`]) already says so.

use std::collections::HashMap;

use gpui::Keystroke;

use crate::file::{KeymapAction, KeymapSection};
use crate::layer::KeymapLayer;
use crate::lines::SourceLines;

/// One of the meanings given to a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictEntry {
    /// What the binding does as written: `namespace::Name`, `namespace::Name {data}` or `null`.
    pub action: String,
    /// Index of the section in the file.
    pub section: usize,
    /// The 1-based line of the binding, when the layer is a file with known lines.
    pub line: Option<usize>,
}

/// Keystrokes bound more than once, to different things, in one context of one layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeymapConflict {
    /// The layer the duplicates are in.
    pub layer: KeymapLayer,
    /// The context expression of the sections, `None` for everywhere.
    pub context: Option<String>,
    /// The keystrokes, normalised (`ctrl-k ctrl-s`).
    pub keystrokes: String,
    /// The meanings in file order; the last one is in effect.
    pub entries: Vec<ConflictEntry>,
}

impl KeymapConflict {
    /// The entry that is in effect.
    pub fn winner(&self) -> &ConflictEntry {
        self.entries.last().expect("a conflict has two entries")
    }
}

/// Spell keystrokes one way (`shift-ctrl-a` and `ctrl-shift-a` are the same key), falling back to
/// the text with its spaces collapsed when it does not parse (that binding is reported elsewhere).
fn normalise(keystrokes: &str) -> String {
    keystrokes
        .split_whitespace()
        .map(|part| Keystroke::parse(part).map_or_else(|_| part.to_owned(), |k| k.unparse()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn describe(action: &KeymapAction) -> String {
    match action {
        KeymapAction::Unbind => "null".to_owned(),
        KeymapAction::Action { name, data: None } => name.clone(),
        KeymapAction::Action {
            name,
            data: Some(data),
        } => format!("{name} {data}"),
    }
}

/// The duplicates among `sections` of `layer`, in the order their keys first appear.
pub fn find_conflicts(
    layer: KeymapLayer,
    sections: &[(usize, KeymapSection)],
    lines: &SourceLines,
) -> Vec<KeymapConflict> {
    let mut order: Vec<(Option<String>, String)> = Vec::new();
    let mut groups: HashMap<(Option<String>, String), Vec<ConflictEntry>> = HashMap::new();
    for (index, section) in sections {
        let context = section
            .context_expr()
            .map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "));
        for (keystrokes, value) in &section.bindings {
            let Ok(action) = KeymapAction::from_json(value) else {
                continue;
            };
            let key = (context.clone(), normalise(keystrokes));
            let entry = ConflictEntry {
                action: describe(&action),
                section: *index,
                line: lines.binding(*index, keystrokes),
            };
            match groups.get_mut(&key) {
                Some(entries) => entries.push(entry),
                None => {
                    order.push(key.clone());
                    groups.insert(key, vec![entry]);
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|key| {
            let entries = groups.remove(&key)?;
            let differs = entries.iter().any(|e| e.action != entries[0].action);
            differs.then(|| KeymapConflict {
                layer,
                context: key.0,
                keystrokes: key.1,
                entries,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::parse_keymap;

    fn conflicts(text: &str) -> Vec<KeymapConflict> {
        let parsed = parse_keymap(text, KeymapLayer::User).unwrap();
        find_conflicts(KeymapLayer::User, &parsed.sections, &parsed.lines)
    }

    #[test]
    fn the_same_key_and_context_with_two_meanings_is_a_conflict_and_the_last_wins() {
        let found = conflicts(
            r#"[
  {"context": "ResourceTable", "bindings": {"ctrl-x": "a::One"}},
  {"context": "ResourceTable", "bindings": {"ctrl-x": "a::Two", "ctrl-y": "a::One"}}
]"#,
        );
        assert_eq!(found.len(), 1);
        let conflict = &found[0];
        assert_eq!(conflict.context.as_deref(), Some("ResourceTable"));
        assert_eq!(conflict.keystrokes, "ctrl-x");
        assert_eq!(conflict.winner().action, "a::Two");
        let lines: Vec<_> = conflict.entries.iter().map(|e| e.line).collect();
        assert_eq!(lines, [Some(2), Some(3)]);
    }

    #[test]
    fn different_contexts_the_same_action_and_other_spellings_are_handled() {
        // Other context: no conflict. Same action twice: harmless. `shift-ctrl-z` is `ctrl-shift-z`.
        let found = conflicts(
            r#"[
  {"context": "A", "bindings": {"ctrl-x": "a::One"}},
  {"context": "B", "bindings": {"ctrl-x": "a::Two"}},
  {"bindings": {"ctrl-q": "a::One"}},
  {"bindings": {"ctrl-q": "a::One"}},
  {"bindings": {"ctrl-shift-z": "a::One"}},
  {"bindings": {"shift-ctrl-z": ["a::One", {"n": 1}]}}
]"#,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].keystrokes, "ctrl-shift-z");
        assert_eq!(found[0].winner().action, r#"a::One {"n":1}"#);
    }

    #[test]
    fn a_null_after_a_binding_is_a_conflict_too() {
        let found =
            conflicts(r#"[{"bindings": {"ctrl-x": "a::One"}}, {"bindings": {"ctrl-x": null}}]"#);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].winner().action, "null");
    }
}
