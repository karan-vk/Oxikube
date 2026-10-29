//! One object of the plan as the table shows it.

use std::time::Duration;

use oxikube_app::apply::ObjectStatus;
use oxikube_domain::apply::PlanItem;
use serde_json::Value;

/// One row: an object of the plan, what its dry run said, and what the apply did.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Its index in the plan (apply order).
    pub index: usize,
    /// The kind.
    pub kind: String,
    /// The name.
    pub name: String,
    /// The namespace it goes to; `None` for a cluster-scoped kind.
    pub namespace: Option<String>,
    /// The file (or `kustomize:<dir>`) it came from.
    pub source: String,
    /// The 1-based line of its document in that source.
    pub line: usize,
    /// The server's dry run of it, once it came in.
    pub dry: Option<ObjectStatus>,
    /// What the apply did, once it did it.
    pub done: Option<ObjectStatus>,
    /// How long into the run it was decided.
    pub elapsed: Option<Duration>,
}

impl Row {
    /// The row of `item` (the object's text is not kept: it may be a Secret).
    pub fn of(index: usize, item: &PlanItem) -> Self {
        let text = |pointer: &str| {
            item.doc
                .json
                .pointer(pointer)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let namespace = text("/metadata/namespace");
        Self {
            index,
            kind: text("/kind"),
            name: text("/metadata/name"),
            namespace: (!namespace.is_empty()).then_some(namespace),
            source: item.doc.source.as_str().to_owned(),
            line: item.doc.line,
            dry: None,
            done: None,
            elapsed: None,
        }
    }

    /// What the table shows: the apply's answer, else the dry run's.
    pub fn shown(&self) -> Option<&ObjectStatus> {
        self.done.as_ref().or(self.dry.as_ref())
    }

    /// Whether the object failed or was skipped, in the apply or (before it) the dry run.
    pub fn failed(&self) -> bool {
        self.shown().is_some_and(ObjectStatus::is_failure)
    }

    /// "ConfigMap shop/web" or "Namespace shop".
    pub fn label(&self) -> String {
        match &self.namespace {
            Some(namespace) => format!("{} {namespace}/{}", self.kind, self.name),
            None => format!("{} {}", self.kind, self.name),
        }
    }
}
