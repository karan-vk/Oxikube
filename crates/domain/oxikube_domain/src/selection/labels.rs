//! The label editor's model: [`LabelEdit`]s, their validation, the merge patch and the preview.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{OxiError, OxiResult};

/// How many edits one `selection::Label` can carry.
pub const MAX_EDITS: usize = 64;

/// Which map of `metadata` the editor changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelField {
    /// `metadata.labels`: what selectors match.
    #[default]
    Labels,
    /// `metadata.annotations`: free-form notes and tool settings.
    Annotations,
}

impl LabelField {
    /// The key of the map in `metadata`.
    pub const fn key(self) -> &'static str {
        match self {
            LabelField::Labels => "labels",
            LabelField::Annotations => "annotations",
        }
    }

    /// The word for a dialog: `label` or `annotation`.
    pub const fn noun(self) -> &'static str {
        match self {
            LabelField::Labels => "label",
            LabelField::Annotations => "annotation",
        }
    }
}

/// One change of the editor: set `key` to `value`, or remove it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LabelEdit {
    /// The key (`app.kubernetes.io/name`).
    pub key: String,
    /// The new value; `None` removes the key.
    pub value: Option<String>,
}

impl LabelEdit {
    /// Sets `key` to `value` (adds it, or changes it where it exists).
    pub fn set(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: Some(value.into()),
        }
    }

    /// Removes `key`.
    pub fn remove(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: None,
        }
    }
}

/// Checks `edits` the way the API server will, so a wrong key fails once in the dialog and never
/// once per object.
///
/// A key is `name` or `prefix/name`: the name is 1 to 63 characters of letters, digits, `-`, `_`
/// and `.`, starting and ending with a letter or digit; the prefix is a DNS subdomain of at most
/// 253 characters. A label value follows the name's rules and may be empty; an annotation value
/// is free text. A key may appear once.
///
/// # Errors
///
/// `Validation` naming the first problem.
pub fn validate_edits(field: LabelField, edits: &[LabelEdit]) -> OxiResult<()> {
    if edits.is_empty() {
        return Err(OxiError::validation(format!(
            "there is no {} to change",
            field.noun()
        )));
    }
    if edits.len() > MAX_EDITS {
        return Err(OxiError::validation(format!(
            "at most {MAX_EDITS} {}s can change at once",
            field.noun()
        )));
    }
    let mut seen = BTreeSet::new();
    for edit in edits {
        key_problem(&edit.key).map_or(Ok(()), |problem| {
            Err(OxiError::validation(format!(
                "{} key {:?}: {problem}",
                field.noun(),
                edit.key
            )))
        })?;
        if !seen.insert(edit.key.as_str()) {
            return Err(OxiError::validation(format!(
                "the {} key {:?} appears twice",
                field.noun(),
                edit.key
            )));
        }
        if let (LabelField::Labels, Some(value)) = (field, &edit.value)
            && let Some(problem) = label_value_problem(value)
        {
            return Err(OxiError::validation(format!(
                "value of label {:?}: {problem}",
                edit.key
            )));
        }
    }
    Ok(())
}

fn key_problem(key: &str) -> Option<&'static str> {
    if key.is_empty() {
        return Some("is empty");
    }
    let (prefix, name) = match key.rsplit_once('/') {
        Some((prefix, name)) => (Some(prefix), name),
        None => (None, key),
    };
    if let Some(prefix) = prefix
        && !is_dns_subdomain(prefix)
    {
        return Some("the prefix before / must be a lowercase DNS name of at most 253 characters");
    }
    if name.is_empty() || name.len() > 63 || !is_name(name) {
        return Some(
            "the name must be 1 to 63 letters, digits, '-', '_' or '.', starting and ending with \
             a letter or digit",
        );
    }
    None
}

fn label_value_problem(value: &str) -> Option<&'static str> {
    if value.is_empty() || (value.len() <= 63 && is_name(value)) {
        None
    } else {
        Some(
            "must be empty or at most 63 letters, digits, '-', '_' or '.', starting and ending \
             with a letter or digit",
        )
    }
}

fn is_name(text: &str) -> bool {
    let bytes = text.as_bytes();
    let edge = |b: u8| b.is_ascii_alphanumeric();
    !bytes.is_empty()
        && edge(bytes[0])
        && edge(bytes[bytes.len() - 1])
        && bytes
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn is_dns_subdomain(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 253
        && text.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

/// How many of `edits` set a key and how many remove one: `(sets, removes)`.
pub fn edit_counts(edits: &[LabelEdit]) -> (usize, usize) {
    let sets = edits.iter().filter(|edit| edit.value.is_some()).count();
    (sets, edits.len() - sets)
}

/// The JSON merge patch (RFC 7386) that applies `edits` to `field`:
/// `{"metadata":{"labels":{"app":"web","old":null}}}`. A `null` removes the key (or does nothing
/// where the object never had it), a string sets it, every other key is left alone.
pub fn label_merge_patch(field: LabelField, edits: &[LabelEdit]) -> Value {
    let mut map = Map::new();
    for edit in edits {
        map.insert(
            edit.key.clone(),
            edit.value.clone().map_or(Value::Null, Value::String),
        );
    }
    json!({ "metadata": { field.key(): Value::Object(map) } })
}

/// One line of a [`LabelPreview`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelPreview {
    /// The key stays as it is.
    Kept {
        /// The key.
        key: String,
        /// Its value.
        value: String,
    },
    /// The key is new.
    Added {
        /// The key.
        key: String,
        /// Its value.
        value: String,
    },
    /// The key has another value.
    Changed {
        /// The key.
        key: String,
        /// The value it has.
        from: String,
        /// The value it gets.
        to: String,
    },
    /// The key goes.
    Removed {
        /// The key.
        key: String,
        /// The value it had.
        value: String,
    },
}

impl LabelPreview {
    /// The key of the line.
    pub fn key(&self) -> &str {
        match self {
            LabelPreview::Kept { key, .. }
            | LabelPreview::Added { key, .. }
            | LabelPreview::Changed { key, .. }
            | LabelPreview::Removed { key, .. } => key,
        }
    }

    /// Whether the line is a change (not a key that stays).
    pub fn is_change(&self) -> bool {
        !matches!(self, LabelPreview::Kept { .. })
    }
}

/// What a sample object's labels (or annotations) become under `edits`: every key of both, sorted,
/// each marked kept, added, changed or removed. A removal of a key the object lacks shows nothing.
pub fn preview_edits<'a>(
    current: impl IntoIterator<Item = (&'a str, &'a str)>,
    edits: &[LabelEdit],
) -> Vec<LabelPreview> {
    let current: BTreeMap<&str, &str> = current.into_iter().collect();
    let edited: BTreeMap<&str, &Option<String>> = edits
        .iter()
        .map(|edit| (edit.key.as_str(), &edit.value))
        .collect();
    let mut keys: BTreeSet<&str> = current.keys().copied().collect();
    keys.extend(edited.keys().copied());
    keys.into_iter()
        .filter_map(|key| {
            let key_owned = key.to_owned();
            Some(match (current.get(key), edited.get(key)) {
                (Some(&value), None) => LabelPreview::Kept {
                    key: key_owned,
                    value: value.to_owned(),
                },
                (None, Some(Some(value))) => LabelPreview::Added {
                    key: key_owned,
                    value: value.clone(),
                },
                (Some(&from), Some(Some(to))) if from == to => LabelPreview::Kept {
                    key: key_owned,
                    value: from.to_owned(),
                },
                (Some(&from), Some(Some(to))) => LabelPreview::Changed {
                    key: key_owned,
                    from: from.to_owned(),
                    to: to.clone(),
                },
                (Some(&value), Some(None)) => LabelPreview::Removed {
                    key: key_owned,
                    value: value.to_owned(),
                },
                (None, Some(None)) | (None, None) => return None,
            })
        })
        .collect()
}

/// The warning the label editor shows before it changes labels (not annotations): selectors match
/// labels, so a change can detach objects from what selects them. `None` for annotations.
pub fn selector_warning(field: LabelField) -> Option<&'static str> {
    match field {
        LabelField::Labels => Some(
            "Services, ReplicaSets and other selectors match labels: changing or removing one on \
             a Pod can detach it from its Service or its controller, which then starts a \
             replacement.",
        ),
        LabelField::Annotations => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_patch_sets_changes_and_removes_with_null() {
        let patch = label_merge_patch(
            LabelField::Labels,
            &[
                LabelEdit::set("app", "web"),
                LabelEdit::set("tier", "backend"),
                LabelEdit::remove("old"),
            ],
        );
        assert_eq!(
            patch,
            json!({"metadata": {"labels": {"app": "web", "tier": "backend", "old": null}}})
        );
    }

    #[test]
    fn annotations_use_the_same_patch_under_their_own_key() {
        let patch = label_merge_patch(
            LabelField::Annotations,
            &[
                LabelEdit::set("example.io/note", "has spaces and: punctuation"),
                LabelEdit::remove("stale"),
            ],
        );
        assert_eq!(
            patch,
            json!({"metadata": {"annotations": {
                "example.io/note": "has spaces and: punctuation", "stale": null
            }}})
        );
    }

    #[test]
    fn keys_follow_the_apiserver_rules() {
        let ok = |key: &str| validate_edits(LabelField::Labels, &[LabelEdit::set(key, "v")]);
        assert!(ok("app").is_ok());
        assert!(ok("app.kubernetes.io/name").is_ok());
        assert!(ok("a-b_c.d").is_ok());
        for bad in [
            "",
            "-a",
            "a-",
            "a b",
            "UPPER.case/name",
            "/name",
            "a/",
            "x".repeat(64).as_str(),
        ] {
            assert!(ok(bad).is_err(), "{bad:?} should be refused");
        }
        let long_prefix = format!("{}/name", "a".repeat(254));
        assert!(ok(&long_prefix).is_err());
    }

    #[test]
    fn label_values_are_restricted_but_annotation_values_are_not() {
        let set = |field, value: &str| validate_edits(field, &[LabelEdit::set("k", value)]);
        assert!(set(LabelField::Labels, "").is_ok());
        assert!(set(LabelField::Labels, "v1.2_x").is_ok());
        assert!(set(LabelField::Labels, "has space").is_err());
        assert!(set(LabelField::Labels, "-edge").is_err());
        assert!(set(LabelField::Labels, &"x".repeat(64)).is_err());
        assert!(
            set(
                LabelField::Annotations,
                "any text: { \"json\": true }\nnext line"
            )
            .is_ok()
        );
    }

    #[test]
    fn a_key_twice_and_no_edits_are_refused() {
        let twice = [LabelEdit::set("a", "1"), LabelEdit::remove("a")];
        assert!(validate_edits(LabelField::Labels, &twice).is_err());
        assert!(validate_edits(LabelField::Labels, &[]).is_err());
        let many: Vec<LabelEdit> = (0..=MAX_EDITS)
            .map(|i| LabelEdit::set(format!("k{i}"), "v"))
            .collect();
        assert!(validate_edits(LabelField::Labels, &many).is_err());
    }

    #[test]
    fn the_preview_marks_every_key() {
        let current = [
            ("app", "web"),
            ("tier", "frontend"),
            ("old", "1"),
            ("keep", "k"),
        ];
        let preview = preview_edits(
            current,
            &[
                LabelEdit::set("tier", "backend"),
                LabelEdit::set("new", "n"),
                LabelEdit::remove("old"),
                LabelEdit::remove("never-there"),
                LabelEdit::set("keep", "k"),
            ],
        );
        assert_eq!(
            preview,
            vec![
                LabelPreview::Kept {
                    key: "app".into(),
                    value: "web".into()
                },
                LabelPreview::Kept {
                    key: "keep".into(),
                    value: "k".into()
                },
                LabelPreview::Added {
                    key: "new".into(),
                    value: "n".into()
                },
                LabelPreview::Removed {
                    key: "old".into(),
                    value: "1".into()
                },
                LabelPreview::Changed {
                    key: "tier".into(),
                    from: "frontend".into(),
                    to: "backend".into()
                },
            ]
        );
        assert!(preview[2].is_change() && !preview[0].is_change());
    }

    #[test]
    fn only_labels_warn_about_selectors() {
        assert!(selector_warning(LabelField::Labels).is_some());
        assert!(selector_warning(LabelField::Annotations).is_none());
    }
}
