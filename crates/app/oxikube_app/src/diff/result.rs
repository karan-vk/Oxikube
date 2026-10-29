//! [`ResourceDiff`]: what a comparison of two objects found.

use oxikube_domain::ids::ResourceRef;
use serde_json::{Value, json};

use super::changes::Changes;
use super::keys::KeyDiff;

/// One side of a comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffSide {
    /// Which object it is.
    pub target: ResourceRef,
    /// The cluster's name as the user knows it (the session's title), for display; the cluster
    /// id when the service could not read one.
    pub cluster_title: String,
    /// The object as the comparison saw it (normalised; a Secret's values are fingerprints), or
    /// `None` when it does not exist.
    pub object: Option<Value>,
}

impl DiffSide {
    /// `staging · ConfigMap shop/web`: the object as a heading.
    pub fn label(&self) -> String {
        let target = &self.target;
        let place = match target.namespace() {
            Some(namespace) => format!("{namespace}/{}", target.name),
            None => target.name.to_string(),
        };
        format!("{} · {} {place}", self.cluster_title, target.gvk.kind)
    }

    /// Whether the object exists.
    pub fn exists(&self) -> bool {
        self.object.is_some()
    }
}

/// The result of comparing two objects of one kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceDiff {
    /// The left side (the object the comparison started from).
    pub left: DiffSide,
    /// The right side (the object it was compared with).
    pub right: DiffSide,
    /// The paths that differ.
    pub changes: Changes,
    /// The keys of a ConfigMap or Secret, with how each compares; empty for other kinds.
    pub keys: Vec<KeyDiff>,
    /// Whether `status` was compared.
    pub include_status: bool,
    /// Whether the name and namespace were compared.
    pub compare_identity: bool,
}

impl ResourceDiff {
    /// Whether the two sides are the same after normalising (both exist and nothing differs).
    pub fn is_identical(&self) -> bool {
        self.left.exists() && self.right.exists() && self.changes.is_empty()
    }

    /// The summary as lines: a headline, then one line per changed path (a ConfigMap's or
    /// Secret's keys are paths under `data`, so they are listed here). It holds no object and no
    /// Secret value, so it is what an agent's tool call returns.
    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = vec![self.headline()];
        if self.left.exists() && self.right.exists() {
            lines.extend(self.changes.paths.iter().map(|change| change.line()));
        } else {
            let top = self.changes.paths.len();
            lines.push(format!("{top} top-level fields only on one side"));
        }
        if self.changes.omitted > 0 {
            lines.push(format!("... and {} more paths", self.changes.omitted));
        }
        lines
    }

    /// One line per key of a ConfigMap or Secret, `same` ones included: `data.password: changed`,
    /// `data.username: same`. Empty for other kinds. No value is in any line.
    pub fn key_lines(&self) -> Vec<String> {
        self.keys
            .iter()
            .map(|k| format!("{}.{}: {}", k.field, k.key, k.status.label()))
            .collect()
    }

    /// [`summary_lines`](Self::summary_lines) as one text.
    pub fn summary_text(&self) -> String {
        self.summary_lines().join("\n")
    }

    /// The summary as the tool's structured result.
    pub fn summary_data(&self) -> Value {
        json!({
            "left": self.left.label(),
            "right": self.right.label(),
            "left_exists": self.left.exists(),
            "right_exists": self.right.exists(),
            "identical": self.is_identical(),
            "changes": self.changes.paths.iter().map(|c| c.line()).collect::<Vec<_>>(),
            "omitted": self.changes.omitted,
            "keys": self.key_lines(),
        })
    }

    fn headline(&self) -> String {
        match (self.left.exists(), self.right.exists()) {
            (true, true) if self.changes.is_empty() => {
                format!(
                    "{} and {} are the same",
                    self.left.label(),
                    self.right.label()
                )
            }
            (true, true) => format!(
                "{} differs from {} in {} {}",
                self.left.label(),
                self.right.label(),
                self.changes.total(),
                if self.changes.total() == 1 {
                    "path"
                } else {
                    "paths"
                }
            ),
            (false, true) => format!(
                "{} does not exist; {} is all new",
                self.left.label(),
                self.right.label()
            ),
            (true, false) => format!(
                "{} does not exist; {} is all new",
                self.right.label(),
                self.left.label()
            ),
            (false, false) => "neither object exists".to_owned(),
        }
    }
}
