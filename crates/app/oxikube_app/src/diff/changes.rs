//! The changed-path summary: what differs between two normalised objects, one line per path
//! (`spec.replicas: 2 -> 3`), added and removed keys included.
//!
//! The summary is computed from normalised objects, where a Secret's values are fingerprints, and
//! [`changes`] never prints anything under a Secret's `data` / `stringData`: those paths read
//! `changed`, `added` or `removed` and nothing else.

use serde_json::Value;

use super::normalize::{SECRET_FIELDS, is_secret};

/// The most paths one summary lists; the rest is counted in [`Changes::omitted`].
pub const MAX_CHANGES: usize = 1000;

/// A string longer than this (or with several lines) is summarised by its size.
const INLINE_LIMIT: usize = 48;

/// What happened to one path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// Only the right side has it.
    Added,
    /// Only the left side has it.
    Removed,
    /// Both have it, with different values.
    Changed,
}

/// One path that differs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathChange {
    /// The path from the object's root: `spec.template.spec.containers[0].image`.
    pub path: String,
    /// What happened to it.
    pub kind: ChangeKind,
    /// The left value as text; `None` when it is not shown (Secret values) or absent.
    pub from: Option<String>,
    /// The right value as text; `None` when it is not shown or absent.
    pub to: Option<String>,
}

impl PathChange {
    /// The line the summary shows: `spec.replicas: 2 -> 3`, `data.new: added "x"`,
    /// `data.old: removed`.
    pub fn line(&self) -> String {
        match (&self.kind, &self.from, &self.to) {
            (ChangeKind::Changed, Some(from), Some(to)) => {
                format!("{}: {from} -> {to}", self.path)
            }
            (ChangeKind::Changed, ..) => format!("{}: changed", self.path),
            (ChangeKind::Added, _, Some(to)) => format!("{}: added {to}", self.path),
            (ChangeKind::Added, ..) => format!("{}: added", self.path),
            (ChangeKind::Removed, Some(from), _) => format!("{}: removed (was {from})", self.path),
            (ChangeKind::Removed, ..) => format!("{}: removed", self.path),
        }
    }
}

/// The paths that differ, in document order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// The listed paths (at most [`MAX_CHANGES`]).
    pub paths: Vec<PathChange>,
    /// How many more paths differ than are listed.
    pub omitted: usize,
}

impl Changes {
    /// Whether nothing differs.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// How many paths differ, listed or not.
    pub fn total(&self) -> usize {
        self.paths.len() + self.omitted
    }

    fn push(&mut self, change: PathChange) {
        if self.paths.len() < MAX_CHANGES {
            self.paths.push(change);
        } else {
            self.omitted += 1;
        }
    }
}

/// The paths where `left` and `right` (both normalised) differ. `secret` says the objects are
/// Secrets, whose `data` / `stringData` values are never shown.
pub fn changes(left: &Value, right: &Value) -> Changes {
    let secret = is_secret(left) || is_secret(right);
    let mut out = Changes::default();
    walk(left, right, &mut Path::default(), secret, &mut out);
    out
}

#[derive(Default)]
struct Path {
    text: String,
    /// Depth and field name of the Secret value field we are inside, if any.
    parts: Vec<String>,
}

impl Path {
    fn key(&mut self, key: &str) -> usize {
        let mark = self.text.len();
        if key.contains(['.', '/', '[', ']', '"', ' ']) {
            self.text.push_str(&format!("[\"{key}\"]"));
        } else {
            if !self.text.is_empty() {
                self.text.push('.');
            }
            self.text.push_str(key);
        }
        self.parts.push(key.to_owned());
        mark
    }

    fn index(&mut self, index: usize) -> usize {
        let mark = self.text.len();
        self.text.push_str(&format!("[{index}]"));
        self.parts.push(index.to_string());
        mark
    }

    fn pop(&mut self, mark: usize) {
        self.text.truncate(mark);
        self.parts.pop();
    }

    /// Whether the path is under a Secret's value fields (`data.x`, `stringData.x`).
    fn hides_values(&self, secret: bool) -> bool {
        secret && self.parts.len() >= 2 && SECRET_FIELDS.contains(&self.parts[0].as_str())
    }
}

fn walk(left: &Value, right: &Value, path: &mut Path, secret: bool, out: &mut Changes) {
    match (left, right) {
        (Value::Object(l), Value::Object(r)) => {
            for (key, lv) in l {
                let mark = path.key(key);
                match r.get(key) {
                    Some(rv) => walk(lv, rv, path, secret, out),
                    None => removed(lv, path, secret, out),
                }
                path.pop(mark);
            }
            for (key, rv) in r {
                if !l.contains_key(key) {
                    let mark = path.key(key);
                    added(rv, path, secret, out);
                    path.pop(mark);
                }
            }
        }
        (Value::Array(l), Value::Array(r)) => {
            for index in 0..l.len().max(r.len()) {
                let mark = path.index(index);
                match (l.get(index), r.get(index)) {
                    (Some(lv), Some(rv)) => walk(lv, rv, path, secret, out),
                    (Some(lv), None) => removed(lv, path, secret, out),
                    (None, Some(rv)) => added(rv, path, secret, out),
                    (None, None) => {}
                }
                path.pop(mark);
            }
        }
        _ if left == right => {}
        _ => {
            let hidden = path.hides_values(secret);
            out.push(PathChange {
                path: path.text.clone(),
                kind: ChangeKind::Changed,
                from: (!hidden).then(|| show(left)),
                to: (!hidden).then(|| show(right)),
            });
        }
    }
}

fn added(value: &Value, path: &Path, secret: bool, out: &mut Changes) {
    out.push(PathChange {
        path: path.text.clone(),
        kind: ChangeKind::Added,
        from: None,
        to: (!path.hides_values(secret)).then(|| show(value)),
    });
}

fn removed(value: &Value, path: &Path, secret: bool, out: &mut Changes) {
    out.push(PathChange {
        path: path.text.clone(),
        kind: ChangeKind::Removed,
        from: (!path.hides_values(secret)).then(|| show(value)),
        to: None,
    });
}

/// A value as one short line.
fn show(value: &Value) -> String {
    match value {
        Value::String(text) if text.len() <= INLINE_LIMIT && !text.contains('\n') => {
            format!("{text:?}")
        }
        Value::String(text) => format!("({} bytes)", text.len()),
        Value::Object(map) => format!("({} keys)", map.len()),
        Value::Array(items) => format!("({} items)", items.len()),
        other => other.to_string(),
    }
}
