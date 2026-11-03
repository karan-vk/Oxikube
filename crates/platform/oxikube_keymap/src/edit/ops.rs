//! [`KeybindUpdateOperation`]: the three edits of the user's `keymap.json`, applied to its text
//! with the comment-preserving array helpers of `oxikube_settings::json_edit`.
//!
//! The operations follow the design of Zed's keymap editor (written from scratch here):
//! - [`KeybindUpdateOperation::Add`] puts a binding in the section of its context, or appends a
//!   section for it, so the file keeps one section per context;
//! - [`KeybindUpdateOperation::Replace`] edits a user binding in place (a key renamed, its value
//!   set), or, for a default binding, adds the override and unbinds the old keys with `null`;
//! - [`KeybindUpdateOperation::Remove`] deletes a user binding (and its section once empty), or
//!   adds a `null` that unbinds a default one.
//!
//! Default and base keymaps are never edited: every operation writes the user's file.

use oxikube_settings::json_edit::{
    infer_json_indent_size, replace_top_level_array_value_in_json_text,
};
use serde_json::{Value, json};

use super::text::{
    append_section, apply, find_binding, insert_first_binding, section_for, sections,
};
use crate::conflicts::normalise;
use crate::file::KeymapAction;
use crate::layer::KeymapLayer;

/// A binding as it is to be written: `keystrokes` in `context` do `action` (`Unbind` writes
/// `null`).
#[derive(Clone, Debug, PartialEq)]
pub struct KeybindTarget {
    /// The context expression; `None` for everywhere.
    pub context: Option<String>,
    /// The keystrokes, space separated, spelled as they are to appear in the file.
    pub keystrokes: String,
    /// What the keys do.
    pub action: KeymapAction,
}

/// Where a binding is: its section's context, its keystrokes and its layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingLocation {
    /// The section's context expression; `None` for everywhere.
    pub context: Option<String>,
    /// The keystrokes (any spelling of the same keys finds it).
    pub keystrokes: String,
    /// The layer the binding is in.
    pub layer: KeymapLayer,
}

/// One edit of the user's `keymap.json`: add, replace or remove a binding (module docs of
/// `oxikube_keymap::edit::ops`).
#[derive(Clone, Debug, PartialEq)]
pub enum KeybindUpdateOperation {
    /// Write `0` into the section of its context (appending one when there is none).
    Add(KeybindTarget),
    /// Make the binding at `target` be `with` instead.
    Replace {
        /// The binding being changed.
        target: BindingLocation,
        /// What it becomes.
        with: KeybindTarget,
    },
    /// Take the binding at `target` out of force.
    Remove {
        /// The binding.
        target: BindingLocation,
    },
}

/// Why an edit could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The user's file is not a list of sections, so nothing can be placed in it.
    InvalidFile(String),
    /// The user binding to change or remove is not in the file (it changed since it was listed).
    NotFound {
        /// Its context.
        context: Option<String>,
        /// Its keystrokes.
        keystrokes: String,
    },
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFile(message) => {
                write!(
                    f,
                    "keymap.json cannot be edited until it is fixed: {message}"
                )
            }
            Self::NotFound {
                context,
                keystrokes,
            } => write!(
                f,
                "keymap.json has no binding of `{keystrokes}` in {}; it changed since it was read",
                context.as_deref().unwrap_or("the everywhere section")
            ),
        }
    }
}

impl std::error::Error for EditError {}

/// The JSON a binding's action is written as: `null`, `"name"` or `["name", data]`.
pub fn action_value(action: &KeymapAction) -> Value {
    match action {
        KeymapAction::Unbind => Value::Null,
        KeymapAction::Action { name, data: None } => Value::String(name.clone()),
        KeymapAction::Action {
            name,
            data: Some(data),
        } => json!([name, data]),
    }
}

/// A context expression spelled one way (`A&&B` is `A && B`), as the rows show it; blank is
/// `None`. Text that does not parse keeps its own spelling, whitespace collapsed.
pub fn normalise_context(context: Option<&str>) -> Option<String> {
    let text = context.map(str::trim).filter(|text| !text.is_empty())?;
    Some(gpui::KeyBindingContextPredicate::parse(text).map_or_else(
        |_| text.split_whitespace().collect::<Vec<_>>().join(" "),
        |predicate| predicate.to_string(),
    ))
}

/// Apply `operations` in order to `text`, the user's `keymap.json`, keeping its comments and
/// formatting. Blank text is an empty keymap.
///
/// # Errors
///
/// [`EditError::InvalidFile`] when the text is not a list of sections, [`EditError::NotFound`]
/// when a user binding to change is not in it. The text is not checked otherwise; run the
/// validator on the result before writing it ([`crate::edit::preview_edit`]).
pub fn apply_operations(
    text: &str,
    operations: &[KeybindUpdateOperation],
) -> Result<String, EditError> {
    let mut text = text.to_owned();
    for operation in operations {
        text = apply_one(&text, operation)?;
    }
    Ok(text)
}

fn apply_one(text: &str, operation: &KeybindUpdateOperation) -> Result<String, EditError> {
    match operation {
        KeybindUpdateOperation::Add(target) => add(text, target),
        KeybindUpdateOperation::Replace { target, with } if target.layer == KeymapLayer::User => {
            replace_user(text, target, with)
        }
        KeybindUpdateOperation::Replace { target, with } => {
            let text = add(text, with)?;
            if same_place(target, with) {
                // The override in the same place already outranks the default.
                return Ok(text);
            }
            add(&text, &unbind_of(target))
        }
        KeybindUpdateOperation::Remove { target } if target.layer == KeymapLayer::User => {
            remove_user(text, target)
        }
        KeybindUpdateOperation::Remove { target } => add(text, &unbind_of(target)),
    }
}

fn unbind_of(target: &BindingLocation) -> KeybindTarget {
    KeybindTarget {
        context: target.context.clone(),
        keystrokes: target.keystrokes.clone(),
        action: KeymapAction::Unbind,
    }
}

fn same_place(target: &BindingLocation, with: &KeybindTarget) -> bool {
    normalise_context(target.context.as_deref()) == normalise_context(with.context.as_deref())
        && normalise(&target.keystrokes) == normalise(&with.keystrokes)
}

fn add(text: &str, target: &KeybindTarget) -> Result<String, EditError> {
    let all = sections(text)?;
    let value = action_value(&target.action);
    let tab_size = infer_json_indent_size(text);
    let Some(index) = section_for(&all, target.context.as_deref()) else {
        return Ok(append_section(text, target, value, tab_size));
    };
    // The same keys spelled another way are the same binding: rename the key rather than add a
    // second one GPUI would treat as a duplicate.
    let existing = all
        .iter()
        .find(|(i, _)| *i == index)
        .and_then(|(_, section)| {
            let keys = normalise(&target.keystrokes);
            section
                .bindings
                .keys()
                .find(|key| normalise(key) == keys)
                .cloned()
        });
    let Some(key) = existing else {
        return match insert_first_binding(text, &all, index, &target.keystrokes, &value) {
            Some(edit) => Ok(apply(text, edit)),
            None => Ok(append_section(text, target, value, tab_size)),
        };
    };
    let rename = (key != target.keystrokes).then_some(target.keystrokes.as_str());
    let edit = replace_top_level_array_value_in_json_text(
        text,
        &["bindings", key.as_str()],
        Some(&value),
        rename,
        index,
        tab_size,
    );
    Ok(apply(text, edit))
}

fn replace_user(
    text: &str,
    target: &BindingLocation,
    with: &KeybindTarget,
) -> Result<String, EditError> {
    let all = sections(text)?;
    let Some((index, key)) = find_binding(&all, target.context.as_deref(), &target.keystrokes)
    else {
        return Err(not_found(target));
    };
    let same_context =
        normalise_context(target.context.as_deref()) == normalise_context(with.context.as_deref());
    let new_keys = normalise(&with.keystrokes);
    let clash = all
        .iter()
        .find(|(i, _)| *i == index)
        .is_some_and(|(_, section)| {
            section
                .bindings
                .keys()
                .any(|other| *other != key && normalise(other) == new_keys)
        });
    if !same_context || clash {
        // Elsewhere, or onto keys the section already has: take the old one out, then add.
        let text = remove_user(text, target)?;
        return add(&text, with);
    }
    let rename = (key != with.keystrokes).then_some(with.keystrokes.as_str());
    let edit = replace_top_level_array_value_in_json_text(
        text,
        &["bindings", key.as_str()],
        Some(&action_value(&with.action)),
        rename,
        index,
        infer_json_indent_size(text),
    );
    Ok(apply(text, edit))
}

fn remove_user(text: &str, target: &BindingLocation) -> Result<String, EditError> {
    let all = sections(text)?;
    let Some((index, key)) = find_binding(&all, target.context.as_deref(), &target.keystrokes)
    else {
        return Err(not_found(target));
    };
    let tab_size = infer_json_indent_size(text);
    let last_binding = all
        .iter()
        .find(|(i, _)| *i == index)
        .is_some_and(|(_, section)| section.bindings.len() == 1);
    let edit = if last_binding {
        // A section left with no bindings goes too.
        replace_top_level_array_value_in_json_text(
            text,
            &[] as &[&str],
            None,
            None,
            index,
            tab_size,
        )
    } else {
        replace_top_level_array_value_in_json_text(
            text,
            &["bindings", key.as_str()],
            None,
            None,
            index,
            tab_size,
        )
    };
    Ok(apply(text, edit))
}

fn not_found(target: &BindingLocation) -> EditError {
    EditError::NotFound {
        context: target.context.clone(),
        keystrokes: target.keystrokes.clone(),
    }
}
