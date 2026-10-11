//! What the multi-document apply takes and gives.

use oxikube_domain::ids::ResourceRef;

use crate::apply::change::Change;
use crate::apply::types::{ApplyError, ApplyMethod, ApplyPreview, ApplyRequest};

/// How one document's dry run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum PreviewOutcome {
    /// The server rehearsed it: what it would do is in the preview.
    Ready(ApplyPreview),
    /// The rehearsal needs an object an earlier document creates (its namespace, its custom
    /// resource definition), which does not exist yet. It is rehearsed again, and applied, once
    /// that document has been applied.
    Deferred,
    /// The server (or the guard) refused it. It is not sent.
    Failed(ApplyError),
    /// An earlier document it needs failed. It is not sent.
    Skipped,
}

/// One document and its dry run.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentPreview {
    /// Its place among the documents, from 0.
    pub index: usize,
    /// "ConfigMap default/web" (or "ConfigMap web" while the namespace is unknown, "Document 3"
    /// for text that names no object).
    pub label: String,
    /// What is sent for it.
    pub request: ApplyRequest,
    /// The earlier documents it needs (its Namespace, its definition), in buffer order.
    pub depends_on: Vec<usize>,
    /// How the dry run ended.
    pub outcome: PreviewOutcome,
}

impl DocumentPreview {
    /// "document 2", "documents 1 and 3" or "documents 1, 2 and 4" (numbered from 1) for what it
    /// needs; `None` when it needs nothing.
    pub fn needs_text(&self) -> Option<String> {
        let numbers: Vec<String> = self
            .depends_on
            .iter()
            .map(|d| (d + 1).to_string())
            .collect();
        match numbers.as_slice() {
            [] => None,
            [one] => Some(format!("document {one}")),
            [rest @ .., last] => Some(format!("documents {} and {last}", rest.join(", "))),
        }
    }

    /// The server's rehearsal, when there is one.
    pub fn preview(&self) -> Option<&ApplyPreview> {
        match &self.outcome {
            PreviewOutcome::Ready(preview) => Some(preview),
            _ => None,
        }
    }

    /// What applying it does, when the server rehearsed it.
    pub fn change(&self) -> Option<Change> {
        self.preview().map(ApplyPreview::change)
    }

    /// Whether applying sends it: rehearsed, or deferred until what it needs exists.
    pub fn will_apply(&self) -> bool {
        matches!(
            self.outcome,
            PreviewOutcome::Ready(_) | PreviewOutcome::Deferred
        )
    }
}

/// How many documents of each kind of outcome a [`BatchPreview`] has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchCounts {
    /// Rehearsed, creating an object.
    pub create: usize,
    /// Rehearsed, changing an object.
    pub update: usize,
    /// Rehearsed, changing nothing.
    pub unchanged: usize,
    /// Waiting for an earlier document.
    pub deferred: usize,
    /// Refused by the server or the guard.
    pub failed: usize,
    /// Needing a document that failed.
    pub skipped: usize,
}

/// The dry run of every document, in buffer order: the combined preview. The documents may hold
/// Secret data: only the masked previews are kept beyond the request text, and the `Debug` form
/// of a request leaves its text out.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchPreview {
    /// The documents, in buffer order.
    pub documents: Vec<DocumentPreview>,
}

impl BatchPreview {
    /// How many documents each outcome has.
    pub fn counts(&self) -> BatchCounts {
        let mut counts = BatchCounts::default();
        for document in &self.documents {
            match &document.outcome {
                PreviewOutcome::Ready(preview) => match preview.change() {
                    Change::Create => counts.create += 1,
                    Change::Update => counts.update += 1,
                    Change::Unchanged => counts.unchanged += 1,
                },
                PreviewOutcome::Deferred => counts.deferred += 1,
                PreviewOutcome::Failed(_) => counts.failed += 1,
                PreviewOutcome::Skipped => counts.skipped += 1,
            }
        }
        counts
    }

    /// How many documents applying sends.
    pub fn applying(&self) -> usize {
        self.documents.iter().filter(|d| d.will_apply()).count()
    }
}

/// What happened to one document.
#[derive(Debug, Clone, PartialEq)]
pub enum DocumentStatus {
    /// The object was made.
    Created,
    /// The object was changed.
    Configured,
    /// The object was applied and stayed as it was.
    Unchanged,
    /// Refused or failed; nothing was written for it (or the write failed).
    Failed(ApplyError),
    /// Not sent because a document it needs failed; the text names that document.
    Skipped(String),
}

impl DocumentStatus {
    /// "created", "configured", "unchanged", "failed", "skipped": `kubectl apply`'s words.
    pub fn word(&self) -> &'static str {
        match self {
            DocumentStatus::Created => "created",
            DocumentStatus::Configured => "configured",
            DocumentStatus::Unchanged => "unchanged",
            DocumentStatus::Failed(_) => "failed",
            DocumentStatus::Skipped(_) => "skipped",
        }
    }

    /// Whether the document did not reach the cluster.
    pub fn is_failure(&self) -> bool {
        matches!(self, DocumentStatus::Failed(_) | DocumentStatus::Skipped(_))
    }
}

/// The result for one document.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentResult {
    /// Its place among the documents, from 0.
    pub index: usize,
    /// "ConfigMap default/web".
    pub label: String,
    /// The object, when the document named one.
    pub target: Option<ResourceRef>,
    /// What happened.
    pub status: DocumentStatus,
    /// How it was sent (a fallback means the server could not server-side apply the kind);
    /// `None` when it was not sent.
    pub method: Option<ApplyMethod>,
    /// One line: the server's message for a failure, why for a skip, else what was applied.
    pub message: String,
}

/// The per-document results of an apply, in buffer order.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchReport {
    /// One per document of the preview.
    pub documents: Vec<DocumentResult>,
}

impl BatchReport {
    /// How many documents failed or were skipped.
    pub fn failures(&self) -> usize {
        self.documents
            .iter()
            .filter(|d| d.status.is_failure())
            .count()
    }

    /// "3 applied, 1 failed" (and "1 skipped" when there are).
    pub fn summary(&self) -> String {
        let count =
            |f: fn(&DocumentStatus) -> bool| self.documents.iter().filter(|d| f(&d.status)).count();
        let applied = count(|s| !s.is_failure());
        let failed = count(|s| matches!(s, DocumentStatus::Failed(_)));
        let skipped = count(|s| matches!(s, DocumentStatus::Skipped(_)));
        let mut text = format!("{applied} applied");
        if failed > 0 {
            text.push_str(&format!(", {failed} failed"));
        }
        if skipped > 0 {
            text.push_str(&format!(", {skipped} skipped"));
        }
        text
    }
}
