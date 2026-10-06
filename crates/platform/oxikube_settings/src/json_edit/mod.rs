// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/settings_json/src/settings_json.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! Comment-preserving edits to JSON-with-comments text (vendored from Zed's `settings_json`).
//!
//! [`update_value_in_json_text`] diffs an old and a new [`Value`] key by key and rewrites only
//! the parts of the text whose value changed, so comments, indentation, key order and
//! trailing commas elsewhere in the file survive a GUI edit. The text is located with the
//! tree-sitter JSON grammar, which tolerates comments and trailing commas.
//!
//! Changes from upstream: the `#N` array-index key paths (`handle_possible_array_value` and
//! the top-level array helpers, used by Zed's keymap editor) are not vendored, so arrays are
//! replaced as whole values; `find_value_range_in_json_text` and `parse_json_with_comments`
//! are not vendored ([`crate::jsonc`] parses); `util::RangeExt` is inlined; tree-sitter stays on
//! 0.26 (the version gpui-component's editor links), where `QueryMatch::captures` is a field, as
//! upstream has it; panicking `unwrap`s carry the
//! invariant that makes them unreachable; a missing key in a member-less object is inserted
//! inside its braces (upstream rewrote the whole document at the root, moving a header above
//! `{` inside it and dropping block comments). Formatting helpers live in [`format`].

mod format;
#[cfg(test)]
mod tests;

pub use format::{infer_json_indent_size, to_pretty_json};

use serde_json::Value;
use std::{ops::Range, sync::LazyLock};
use tree_sitter::{Query, StreamingIterator as _};

/// A new tree-sitter parser for JSON.
fn json_parser() -> tree_sitter::Parser {
    let mut parser = tree_sitter::Parser::new();
    // Invariant: the grammar is compiled in and its ABI is checked against the linked
    // tree-sitter at build time by `tree-sitter-language`; this cannot fail at runtime.
    parser
        .set_language(&tree_sitter_json::LANGUAGE.into())
        .expect("tree-sitter-json grammar matches the linked tree-sitter");
    parser
}

/// Apply the difference between `old_value` and `new_value` (found at `key_path`) to `text`.
///
/// Objects are compared key by key so unchanged members keep their formatting and comments;
/// anything else that differs is replaced as a whole. Each edit is applied to `text` and also
/// pushed to `edits` (ranges refer to the text as it was when that edit was made).
pub fn update_value_in_json_text<'a>(
    text: &mut String,
    key_path: &mut Vec<&'a str>,
    tab_size: usize,
    old_value: &'a Value,
    new_value: &'a Value,
    edits: &mut Vec<(Range<usize>, String)>,
) {
    // If the old and new values are both objects, then compare them key by key,
    // preserving the comments and formatting of the unchanged parts. Otherwise,
    // replace the old value with the new value.
    if let (Value::Object(old_object), Value::Object(new_object)) = (old_value, new_value) {
        for (key, old_sub_value) in old_object.iter() {
            key_path.push(key);
            if let Some(new_sub_value) = new_object.get(key) {
                // Key exists in both old and new, recursively update
                update_value_in_json_text(
                    text,
                    key_path,
                    tab_size,
                    old_sub_value,
                    new_sub_value,
                    edits,
                );
            } else {
                // Key was removed from new object, remove the entire key-value pair
                let (range, replacement) =
                    replace_value_in_json_text(text, key_path, 0, None, None);
                text.replace_range(range.clone(), &replacement);
                edits.push((range, replacement));
            }
            key_path.pop();
        }
        for (key, new_sub_value) in new_object.iter() {
            key_path.push(key);
            if !old_object.contains_key(key) {
                update_value_in_json_text(
                    text,
                    key_path,
                    tab_size,
                    &Value::Null,
                    new_sub_value,
                    edits,
                );
            }
            key_path.pop();
        }
    } else if old_value != new_value {
        let mut new_value = new_value.clone();
        if let Some(new_object) = new_value.as_object_mut() {
            new_object.retain(|_, v| !v.is_null());
        }
        let (range, replacement) =
            replace_value_in_json_text(text, key_path, tab_size, Some(&new_value), None);
        text.replace_range(range.clone(), &replacement);
        edits.push((range, replacement));
    }
}

/// `range.contains_inclusive(other)` from Zed's `util::RangeExt`.
fn contains_inclusive(range: &Range<usize>, other: &Range<usize>) -> bool {
    range.start <= other.start && other.end <= range.end
}

/// Compute the edit that sets (or, with `new_value: None`, removes) the value at `key_path`.
///
/// Returns the byte range to replace and its replacement. Missing intermediate objects are
/// created. When `replace_key` is `Some`, an exact match also renames the key.
pub fn replace_value_in_json_text<T: AsRef<str>>(
    text: &str,
    key_path: &[T],
    tab_size: usize,
    new_value: Option<&Value>,
    replace_key: Option<&str>,
) -> (Range<usize>, String) {
    static PAIR_QUERY: LazyLock<Query> = LazyLock::new(|| {
        // Invariant: a constant query over the bundled grammar; covered by every edit test.
        Query::new(
            &tree_sitter_json::LANGUAGE.into(),
            "(pair key: (string) @key value: (_) @value)",
        )
        .expect("Failed to create PAIR_QUERY")
    });

    let mut parser = json_parser();
    // Invariant: `parse` returns `None` only when a timeout or cancellation flag is set; this
    // parser has neither.
    let syntax_tree = parser
        .parse(text, None)
        .expect("tree-sitter parse without timeout or cancellation");

    let mut cursor = tree_sitter::QueryCursor::new();

    let mut depth = 0;
    let mut last_value_range = 0..0;
    let mut first_key_start = None;
    let mut matched_key_start = None;
    let mut existing_value_range = 0..text.len();

    let mut matches = cursor.matches(&PAIR_QUERY, syntax_tree.root_node(), text.as_bytes());
    while let Some(mat) = matches.next() {
        if mat.captures.len() != 2 {
            continue;
        }

        let key_range = mat.captures[0].node.byte_range();
        let value_range = mat.captures[1].node.byte_range();

        // Don't enter sub objects until we find an exact
        // match for the current keypath
        if contains_inclusive(&last_value_range, &value_range) {
            continue;
        }

        last_value_range = value_range.clone();

        if key_range.start > existing_value_range.end {
            break;
        }

        first_key_start.get_or_insert(key_range.start);

        let found_key = text
            .get(key_range.clone())
            .zip(key_path.get(depth))
            .and_then(|(key_text, key_path_value)| {
                serde_json::to_string(key_path_value.as_ref())
                    .ok()
                    .map(|key_path| depth < key_path.len() && key_text == key_path)
            })
            .unwrap_or(false);

        if found_key {
            matched_key_start = Some(key_range.start);
            existing_value_range = value_range;
            // Reset last value range when increasing in depth
            last_value_range = existing_value_range.start..existing_value_range.start;
            depth += 1;

            if depth == key_path.len() {
                break;
            }

            first_key_start = None;
        }
    }

    // We found the exact key we want
    if depth == key_path.len() {
        if let Some(new_value) = new_value {
            let new_val = to_pretty_json(new_value, tab_size, tab_size * depth);
            if let Some(replace_key) = replace_key.and_then(|str| serde_json::to_string(str).ok()) {
                let new_key = format!("{}: ", replace_key);
                if let Some(key_start) = matched_key_start {
                    existing_value_range.start = key_start;
                }
                (existing_value_range, new_key + &new_val)
            } else {
                (existing_value_range, new_val)
            }
        } else {
            let mut removal_start = first_key_start.unwrap_or(existing_value_range.start);
            let mut removal_end = existing_value_range.end;

            if let Some(key_start) = matched_key_start {
                removal_start = key_start;
            }

            let mut removed_comma = false;
            // Look backward for a preceding comma first
            let preceding_text = text.get(0..removal_start).unwrap_or("");
            if let Some(comma_pos) = preceding_text.rfind(',') {
                // Check if there are only whitespace characters between the comma and our key
                let between_comma_and_key = text.get(comma_pos + 1..removal_start).unwrap_or("");
                if between_comma_and_key.trim().is_empty() {
                    removal_start = comma_pos;
                    removed_comma = true;
                }
            }
            if let Some(remaining_text) = text.get(existing_value_range.end..)
                && !removed_comma
            {
                let mut chars = remaining_text.char_indices();
                while let Some((offset, ch)) = chars.next() {
                    if ch == ',' {
                        removal_end = existing_value_range.end + offset + 1;
                        // Also consume whitespace after the comma
                        for (_, next_ch) in chars.by_ref() {
                            if next_ch.is_whitespace() {
                                removal_end += next_ch.len_utf8();
                            } else {
                                break;
                            }
                        }
                        break;
                    } else if !ch.is_whitespace() {
                        break;
                    }
                }
            }
            (removal_start..removal_end, String::new())
        }
    } else if let Some(first_key_start) = first_key_start {
        // We have key paths, construct the sub objects
        let new_key = json_string(key_path[depth].as_ref());
        // We don't have the key, construct the nested objects
        let new_value = construct_json_value(&key_path[(depth + 1)..], new_value);

        let mut row = 0;
        let mut column = 0;
        for (ix, char) in text.char_indices() {
            if ix == first_key_start {
                break;
            }
            if char == '\n' {
                row += 1;
                column = 0;
            } else {
                column += char.len_utf8();
            }
        }

        if row > 0 {
            // depth is 0 based, but division needs to be 1 based.
            let new_val = to_pretty_json(&new_value, column / (depth + 1), column);
            let space = ' ';
            let content = format!("{new_key}: {new_val},\n{space:width$}", width = column);
            (first_key_start..first_key_start, content)
        } else {
            let new_val = new_value.to_string();
            let mut content = format!("{new_key}: {new_val},");
            content.push(' ');
            (first_key_start..first_key_start, content)
        }
    } else {
        // The parent object has no members (or there is no root object yet).
        let new_value = construct_json_value(&key_path[depth..], new_value);
        let new_val = to_pretty_json(&new_value, tab_size, tab_size * depth);
        let parent = if depth == 0 {
            root_object_range(&syntax_tree)
        } else {
            Some(existing_value_range.clone())
        };
        let members = new_val
            .strip_prefix('{')
            .and_then(|val| val.strip_suffix('}'));
        match (parent, members) {
            // Insert the members inside the braces: everything before `{` and after `}`
            // (a file header, say) and every comment inside them stays exactly as it was.
            (Some(parent), Some(members))
                if text[parent.clone()].starts_with('{') && text[parent.clone()].ends_with('}') =>
            {
                let inner = &text[parent.start + 1..parent.end - 1];
                let insert_at = parent.start + 1 + inner.trim_end().len();
                (insert_at..parent.end - 1, members.to_owned())
            }
            // No root object (a blank or comment-only file): keep the text, append one.
            (None, _) if depth == 0 => {
                let kept = text.trim_end().len();
                let separator = if kept == 0 { "" } else { "\n" };
                (kept..text.len(), format!("{separator}{new_val}\n"))
            }
            // The parent is not an object: replace it as a whole.
            _ => (existing_value_range, new_val),
        }
    }
}

/// The byte range of the document's root object, if it has one.
fn root_object_range(syntax_tree: &tree_sitter::Tree) -> Option<Range<usize>> {
    let root = syntax_tree.root_node();
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .find(|node| node.kind() == "object")
        .map(|node| node.byte_range())
}

/// `key` as a quoted, escaped JSON string.
fn json_string(key: &str) -> String {
    // Serialising a `&str` into a `String` buffer cannot fail; `Value::String` does the same
    // escaping without a `Result`.
    Value::String(key.to_owned()).to_string()
}

/// Wrap `new_value` in one object level per key of `key_path` (innermost last).
fn construct_json_value(key_path: &[impl AsRef<str>], new_value: Option<&Value>) -> Value {
    let mut new_value = new_value.cloned().unwrap_or(Value::Null);
    for key in key_path.iter().rev() {
        new_value = serde_json::json!({ key.as_ref().to_string(): new_value });
    }
    new_value
}
