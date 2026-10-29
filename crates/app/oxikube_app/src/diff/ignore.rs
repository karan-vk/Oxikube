//! The ignore list of a diff: JSON-pointer-like paths with `*` wildcards, and the shared handle
//! the `diff.ignore_paths` setting is pushed through.
//!
//! A path is an RFC 6901 pointer (`/metadata/annotations/team`). A key that holds a `/` writes it
//! as `~1` (and a `~` as `~0`), so the annotation `kubectl.kubernetes.io/last-applied-configuration`
//! is `/metadata/annotations/kubectl.kubernetes.io~1last-applied-configuration`. Inside a segment
//! `*` matches any run of characters, so `/metadata/annotations/kubectl.kubernetes.io~1*` ignores
//! every kubectl annotation and `/spec/template/spec/containers/*/resources` ignores the resources
//! of every container (an array index is a segment like any other). A path that does not start
//! with `/` is not a path: it is kept out of the list and reported by [`IgnorePaths::rejected`].

use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::Value;

/// What the setting holds when the user has not set it: the annotations the tooling writes on
/// every apply and rollout, which differ between two environments without meaning anything.
pub const DEFAULT_IGNORE_PATHS: &[&str] = &[
    "/metadata/annotations/kubectl.kubernetes.io~1*",
    "/metadata/annotations/deployment.kubernetes.io~1revision",
];

/// One compiled path: a segment per level.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pattern {
    segments: Vec<String>,
}

/// The compiled ignore list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IgnorePaths {
    patterns: Vec<Pattern>,
    rejected: Vec<String>,
}

impl IgnorePaths {
    /// Compiles `paths`; a path that is not a pointer is skipped and listed in
    /// [`rejected`](Self::rejected).
    pub fn new<S: AsRef<str>>(paths: &[S]) -> Self {
        let mut list = Self::default();
        for path in paths {
            let path = path.as_ref().trim();
            match path.strip_prefix('/') {
                Some(rest) if !rest.is_empty() => list.patterns.push(Pattern {
                    segments: rest.split('/').map(unescape).collect(),
                }),
                _ => list.rejected.push(path.to_owned()),
            }
        }
        list
    }

    /// The default list ([`DEFAULT_IGNORE_PATHS`]).
    pub fn standard() -> Self {
        Self::new(DEFAULT_IGNORE_PATHS)
    }

    /// The paths that were not pointers.
    pub fn rejected(&self) -> &[String] {
        &self.rejected
    }

    /// Whether the list ignores nothing.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Removes every node of `value` that a path names. An object or array that was emptied by
    /// the removal goes too (an object with no annotations left reads like one that had none).
    pub fn prune(&self, value: &mut Value) {
        if self.patterns.is_empty() {
            return;
        }
        let mut path = Vec::new();
        prune_with(value, &mut path, self);
    }

    fn matches(&self, path: &[String]) -> bool {
        self.patterns.iter().any(|pattern| {
            pattern.segments.len() == path.len()
                && pattern
                    .segments
                    .iter()
                    .zip(path)
                    .all(|(segment, key)| glob(segment, key))
        })
    }
}

fn unescape(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

/// Whether `text` matches `pattern`, where `*` stands for any run of characters.
fn glob(pattern: &str, text: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == text;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || text.len() < first.len() + last.len() {
        return false;
    }
    if !text.ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// Prunes `node` (at `path`); returns whether it lost something and is empty now.
fn prune_with(node: &mut Value, path: &mut Vec<String>, list: &IgnorePaths) -> bool {
    let mut touched = false;
    match node {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                path.push(key.clone());
                let drop = list.matches(path)
                    || map
                        .get_mut(&key)
                        .is_some_and(|child| prune_with(child, path, list));
                path.pop();
                if drop {
                    map.shift_remove(&key);
                    touched = true;
                }
            }
            touched && map.is_empty()
        }
        Value::Array(items) => {
            let mut kept = Vec::with_capacity(items.len());
            for (index, mut item) in std::mem::take(items).into_iter().enumerate() {
                path.push(index.to_string());
                let drop = list.matches(path) || prune_with(&mut item, path, list);
                path.pop();
                if drop {
                    touched = true;
                } else {
                    kept.push(item);
                }
            }
            *items = kept;
            touched && items.is_empty()
        }
        _ => false,
    }
}

/// The ignore list in effect, shared between the diff service and whoever watches the setting.
/// Setting it applies to the next diff; nothing restarts.
#[derive(Clone, Debug)]
pub struct IgnoreList {
    current: Arc<RwLock<Arc<IgnorePaths>>>,
}

impl IgnoreList {
    /// A handle holding `paths`.
    pub fn new(paths: IgnorePaths) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(paths))),
        }
    }

    /// The list now.
    pub fn get(&self) -> Arc<IgnorePaths> {
        self.current.read().clone()
    }

    /// Replaces the list; later diffs use it.
    pub fn set(&self, paths: IgnorePaths) {
        *self.current.write() = Arc::new(paths);
    }
}

impl Default for IgnoreList {
    fn default() -> Self {
        Self::new(IgnorePaths::standard())
    }
}
