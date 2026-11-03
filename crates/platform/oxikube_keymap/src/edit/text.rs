//! Text-level helpers of the edit operations: finding a section or a binding in the parsed file,
//! and the two insertions the operations make themselves (a new first binding of a section, a new
//! last section).

use oxikube_settings::json_edit::append_top_level_array_value_in_json_text;
use serde_json::{Map, Value};

use super::ops::{EditError, KeybindTarget, normalise_context};
use crate::conflicts::normalise;
use crate::file::{KeymapSection, parse_keymap};
use crate::layer::KeymapLayer;
use crate::lines::SourceLines;

/// The parsed sections of `text` (`[]` for blank text).
pub(super) fn sections(text: &str) -> Result<Vec<(usize, KeymapSection)>, EditError> {
    parse_keymap(text, KeymapLayer::User)
        .map(|parsed| parsed.sections)
        .map_err(|diagnostic| EditError::InvalidFile(diagnostic.message()))
}

/// The array index of the section for `context` that new bindings go to: the first with the same
/// context, without `use_key_equivalents` (a binding recorded here names keys by position) and
/// with a binding to place the new one beside. An empty section is passed over (a new one is
/// appended instead): the JSON edit mis-indents a key opened inside a list element.
pub(super) fn section_for(
    sections: &[(usize, KeymapSection)],
    context: Option<&str>,
) -> Option<usize> {
    let wanted = normalise_context(context);
    sections
        .iter()
        .find(|(_, section)| {
            !section.use_key_equivalents
                && !section.bindings.is_empty()
                && normalise_context(section.context_expr()) == wanted
        })
        .map(|(index, _)| *index)
}

/// The section holding the binding of `keystrokes` in `context`, and the key as the file spells
/// it. The last such binding is the one in force, so it is the one found.
pub(super) fn find_binding(
    sections: &[(usize, KeymapSection)],
    context: Option<&str>,
    keystrokes: &str,
) -> Option<(usize, String)> {
    let wanted = normalise_context(context);
    let keys = normalise(keystrokes);
    sections
        .iter()
        .rev()
        .filter(|(_, section)| normalise_context(section.context_expr()) == wanted)
        .find_map(|(index, section)| {
            section
                .bindings
                .keys()
                .rev()
                .find(|key| normalise(key) == keys)
                .map(|key| (*index, key.clone()))
        })
}

/// The edit that puts a new binding first in the `index`th section's non-empty `bindings`, on a
/// line of its own with the indentation of the binding it goes above (or before it on the same
/// line, in a one-line object). `None` when the section has no binding to go above.
///
/// The JSON edit can insert the key too, but inside an element of the top-level list it indents
/// the line after the new key twice (it measures the column in the element's text and the list
/// helper then indents again), so new keys of an existing section are placed here instead.
pub(super) fn insert_first_binding(
    text: &str,
    all: &[(usize, KeymapSection)],
    index: usize,
    keystrokes: &str,
    value: &Value,
) -> Option<(std::ops::Range<usize>, String)> {
    let (_, section) = all.iter().find(|(i, _)| *i == index)?;
    let first = section.bindings.keys().next()?;
    let at = SourceLines::scan(text)
        .binding_location(index, first)?
        .span
        .start;
    let line_start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    let indent = &text[line_start..at];
    let separator = if indent.trim().is_empty() {
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        format!("{newline}{indent}")
    } else {
        " ".to_owned()
    };
    let key = serde_json::to_string(keystrokes).ok()?;
    let value = serde_json::to_string(value).ok()?;
    Some((at..at, format!("{key}: {value},{separator}")))
}

/// `text` with a new last section holding only `target` (written as `value`).
pub(super) fn append_section(
    text: &str,
    target: &KeybindTarget,
    value: Value,
    tab_size: usize,
) -> String {
    let mut section = Map::new();
    if let Some(context) = &target.context {
        section.insert("context".to_owned(), Value::String(context.clone()));
    }
    let mut bindings = Map::new();
    bindings.insert(target.keystrokes.clone(), value);
    section.insert("bindings".to_owned(), Value::Object(bindings));
    let edit = append_top_level_array_value_in_json_text(text, &Value::Object(section), tab_size);
    apply(text, edit)
}

pub(super) fn apply(text: &str, (range, replacement): (std::ops::Range<usize>, String)) -> String {
    let mut out = text.to_owned();
    out.replace_range(range, &replacement);
    out
}
