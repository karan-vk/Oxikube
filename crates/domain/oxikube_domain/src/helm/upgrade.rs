//! The upgrade tab's pure parts (E16-S10): what an upgrade changes in the cluster, by object
//! (kind and name), and the values text made safe to show in a diff.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use serde_json::Value;

use super::values::Values;
use crate::redact::is_sensitive_field;

/// One object of a rendered manifest: its kind, name and namespace (when the chart sets one).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectKey {
    /// `Deployment`, `Service`, ...
    pub kind: String,
    /// `metadata.name`.
    pub name: String,
    /// `metadata.namespace`, when the chart sets it.
    pub namespace: Option<String>,
}

impl fmt::Display for ObjectKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.namespace {
            Some(namespace) => write!(f, "{} {namespace}/{}", self.kind, self.name),
            None => write!(f, "{} {}", self.kind, self.name),
        }
    }
}

/// What upgrading changes, object by object: the objects the new manifest adds, the ones it no
/// longer has (Helm deletes them) and the ones whose definition differs. Names by kind, not full
/// object diffs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceChanges {
    /// In the new manifest only.
    pub added: Vec<ObjectKey>,
    /// In the current manifest only.
    pub removed: Vec<ObjectKey>,
    /// In both, with a different definition.
    pub changed: Vec<ObjectKey>,
    /// In both, the same.
    pub unchanged: usize,
}

impl ResourceChanges {
    /// The changes from `current` to `next` (both multi-document manifests). Comments, key order
    /// and the order of documents never count as changes. A document that is not valid YAML is
    /// compared as text, named by its position.
    ///
    /// Parses every document once: call it off the UI thread for a large manifest.
    pub fn between(current: &str, next: &str) -> Self {
        let before = objects(current);
        let after = objects(next);
        let mut out = Self::default();
        for (key, body) in &after {
            match before.get(key) {
                None => out.added.push(key.clone()),
                Some(old) if old != body => out.changed.push(key.clone()),
                Some(_) => out.unchanged += 1,
            }
        }
        out.removed = before
            .keys()
            .filter(|key| !after.contains_key(*key))
            .cloned()
            .collect();
        out
    }

    /// Whether the upgrade touches no object.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// `2 added, 1 removed, 3 changed, 12 unchanged`-style text.
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return format!("No object changes ({} unchanged)", self.unchanged);
        }
        format!(
            "{} added, {} removed, {} changed, {} unchanged",
            self.added.len(),
            self.removed.len(),
            self.changed.len(),
            self.unchanged
        )
    }
}

/// The objects of `manifest` with their parsed bodies (chart hooks excluded).
fn objects(manifest: &str) -> BTreeMap<ObjectKey, Value> {
    let mut found = BTreeMap::new();
    for (position, document) in documents(manifest).into_iter().enumerate() {
        let body = Values::from_yaml(document)
            .map(|values| values.as_json().clone())
            .unwrap_or_else(|_| Value::String(document.to_owned()));
        // `helm get manifest` leaves hooks out, `helm template` renders them: ignore them on both
        // sides so a no-op upgrade reads as no change.
        if body
            .pointer("/metadata/annotations/helm.sh~1hook")
            .is_some()
        {
            continue;
        }
        let field = |path: &[&str]| {
            let mut at = &body;
            for key in path {
                at = at.get(key)?;
            }
            at.as_str().map(str::to_owned)
        };
        let key = match (field(&["kind"]), field(&["metadata", "name"])) {
            (Some(kind), Some(name)) => ObjectKey {
                kind,
                name,
                namespace: field(&["metadata", "namespace"]),
            },
            _ if body.as_object().is_some_and(|map| map.is_empty()) => continue,
            _ => ObjectKey {
                kind: "Document".to_owned(),
                name: format!("#{}", position + 1),
                namespace: None,
            },
        };
        found.insert(key, body);
    }
    found
}

/// The non-blank documents of `manifest`, split on `---` lines.
fn documents(manifest: &str) -> Vec<&str> {
    let mut starts = vec![0usize];
    let mut offset = 0usize;
    for line in manifest.split_inclusive('\n') {
        if (line.starts_with("---") && line[3..].trim().is_empty()) && offset > 0 {
            starts.push(offset);
        }
        offset += line.len();
    }
    starts.push(manifest.len());
    starts
        .windows(2)
        .map(|w| &manifest[w[0]..w[1]])
        .filter(|doc| {
            doc.lines()
                .any(|l| !l.trim().is_empty() && !l.starts_with("---") && !l.starts_with('#'))
        })
        .collect()
}

/// `values` with the value of every secret-looking key (`password`, `token`, `apiKey`, ...)
/// replaced by `(hidden #xxxxxxxx)`, where the digits are a fingerprint: equal values show equal
/// fingerprints, so a diff of two texts still shows that a secret changed but never what it is.
/// Line based. A secret's block scalar (`key: |`) is replaced as a whole, and secret keys inside
/// a one-line flow map (`auth: {password: x}`) are masked; other lines are untouched.
pub fn mask_values_for_diff(values: &str) -> String {
    let mut out = String::with_capacity(values.len());
    let mut lines = values.lines().peekable();
    while let Some(line) = lines.next() {
        let indent = line.len() - line.trim_start().len();
        let body = line[indent..].trim_start_matches("- ");
        let block = body
            .split_once(':')
            .is_some_and(|(_, value)| value.trim().starts_with(['|', '>']));
        if block {
            let mut text = String::new();
            while let Some(next) = lines.peek() {
                let deeper = next.len() - next.trim_start().len() > indent;
                if next.trim().is_empty() || deeper {
                    text.push_str(next);
                    text.push('\n');
                    lines.next();
                } else {
                    break;
                }
            }
            let key = body.split_once(':').map_or("", |(key, _)| key);
            let secret = is_sensitive_field(key.trim().trim_matches(['"', '\'']))
                || text.contains("PRIVATE KEY-----");
            if !secret {
                out.push_str(line);
                out.push('\n');
                out.push_str(&text);
                continue;
            }
            let key_end = line.len() - body.len() + body.find(':').unwrap_or(0);
            out.push_str(&format!(
                "{}: (hidden #{:08x})\n",
                &line[..key_end],
                fingerprint(&text)
            ));
            continue;
        }
        out.push_str(&mask_line(line));
        out.push('\n');
    }
    out
}

fn fingerprint(text: &str) -> u32 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish() as u32
}

/// Masks the secret entries of a one-line flow map `{a: 1, password: x}`.
fn mask_flow(value: &str) -> Option<String> {
    let inner = value.strip_prefix('{')?.strip_suffix('}')?;
    let mut changed = false;
    let parts: Vec<String> = inner
        .split(',')
        .map(|pair| match pair.split_once(':') {
            Some((key, v)) if is_sensitive_field(key.trim().trim_matches(['"', '\''])) => {
                changed = true;
                format!("{key}: (hidden #{:08x})", fingerprint(v.trim()))
            }
            _ => pair.to_owned(),
        })
        .collect();
    changed.then(|| format!("{{{}}}", parts.join(",")))
}

fn mask_line(line: &str) -> String {
    let indent = line.len() - line.trim_start().len();
    let body = line[indent..].trim_start_matches("- ");
    let Some((key, value)) = body.split_once(':') else {
        return line.to_owned();
    };
    let key = key.trim().trim_matches(['"', '\'']);
    let value = value.trim();
    if value.starts_with('{')
        && let Some(masked) = mask_flow(value)
    {
        let prefix_len = line.len() - body.len();
        return format!("{}{key}: {masked}", &line[..prefix_len]);
    }
    let plain = !value.is_empty()
        && !value.starts_with(['|', '>', '[', '{', '#', '&', '*'])
        && !value.eq_ignore_ascii_case("null")
        && !value.eq_ignore_ascii_case("~");
    if !plain || !is_sensitive_field(key) {
        return line.to_owned();
    }
    let prefix_len = line.len() - body.len();
    let key_part = &line[..prefix_len + body.find(':').unwrap_or(0)];
    format!("{key_part}: (hidden #{:08x})", fingerprint(value))
}
