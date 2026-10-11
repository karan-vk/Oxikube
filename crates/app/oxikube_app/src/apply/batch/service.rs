//! `ApplyService::preview_batch` and `apply_batch`.

use futures::StreamExt as _;
use futures::stream;
use oxikube_domain::ids::ResourceRef;

use super::depends::dependencies;
use super::types::{
    BatchPreview, BatchReport, DocumentPreview, DocumentResult, DocumentStatus, PreviewOutcome,
};
use crate::actions::object_label;
use crate::apply::change::Change;
use crate::apply::manifest::Manifest;
use crate::apply::service::ApplyService;
use crate::apply::types::{ApplyError, ApplyMethod, ApplyPreview, ApplyRequest};

/// How many documents are dry-run together.
const DRY_RUNS_AT_ONCE: usize = 8;

/// A document's status, one line, the object it made, how it was sent and the
/// `metadata.resourceVersion` the server stored.
type Outcome = (
    DocumentStatus,
    String,
    Option<ResourceRef>,
    Option<ApplyMethod>,
    Option<String>,
);

impl ApplyService {
    /// The server's dry run of every document of `requests` (one object each, in buffer order),
    /// for the combined preview. Nothing is written. A document that needs an object an earlier one
    /// creates (its namespace, its definition) and so cannot be rehearsed yet is
    /// [`Deferred`](PreviewOutcome::Deferred), and one that needs an earlier document that failed
    /// is [`Skipped`](PreviewOutcome::Skipped).
    pub async fn preview_batch(&self, requests: Vec<ApplyRequest>) -> BatchPreview {
        let manifests: Vec<Option<Manifest>> = requests
            .iter()
            .map(|request| Manifest::parse(&request.manifest).ok())
            .collect();
        let needs = dependencies(&requests, &manifests);
        // A few dry runs at a time: a dump of hundreds of documents is not hundreds of requests
        // at once, and the answers keep the documents' order.
        let answers: Vec<_> = stream::iter(requests.iter().cloned().map(|r| self.preview(r)))
            .buffered(DRY_RUNS_AT_ONCE)
            .collect()
            .await;
        let mut documents: Vec<DocumentPreview> = Vec::with_capacity(requests.len());
        for (index, ((request, answer), depends_on)) in
            requests.into_iter().zip(answers).zip(needs).enumerate()
        {
            let blocked = depends_on.iter().any(|&earlier| {
                matches!(
                    documents[earlier].outcome,
                    PreviewOutcome::Failed(_) | PreviewOutcome::Skipped
                )
            });
            // An earlier document that makes what this one needs: until it has, the server
            // cannot rehearse this one. Any one of them is enough, whichever order they come in.
            let awaited = depends_on.iter().any(|&earlier| {
                matches!(documents[earlier].outcome, PreviewOutcome::Deferred)
                    || documents[earlier].change() == Some(Change::Create)
            });
            let label = manifests[index].as_ref().map_or_else(
                || format!("Document {}", index + 1),
                |m| format!("{} {}", m.gvk.kind, m.name),
            );
            let (label, outcome) = match answer {
                _ if blocked => (label, PreviewOutcome::Skipped),
                Ok(preview) => (preview.label(), PreviewOutcome::Ready(preview)),
                Err(_) if awaited => (label, PreviewOutcome::Deferred),
                Err(error) => (label, PreviewOutcome::Failed(error)),
            };
            documents.push(DocumentPreview {
                index,
                label,
                request,
                depends_on,
                outcome,
            });
        }
        BatchPreview { documents }
    }

    /// Applies what [`preview_batch`](Self::preview_batch) showed, in buffer order, each document
    /// as `resource::Apply` (the preview is the confirmation). A failure does not stop the
    /// rest, as with `kubectl apply`; the documents that need the failed one are skipped. A
    /// deferred document is rehearsed and applied when its turn comes. Documents whose dry run
    /// failed are reported as failed without being sent.
    pub async fn apply_batch(&self, preview: &BatchPreview) -> BatchReport {
        let mut results: Vec<DocumentResult> = Vec::with_capacity(preview.documents.len());
        for document in &preview.documents {
            let cause = document
                .depends_on
                .iter()
                .map(|&earlier| &results[earlier])
                .find(|result| result.status.is_failure());
            let (status, message, target, method, resource_version) =
                match (&document.outcome, cause) {
                    (_, Some(cause)) => skipped(cause),
                    (PreviewOutcome::Skipped, None) => skipped_earlier(document),
                    (PreviewOutcome::Failed(error), None) => failed(error.clone()),
                    (PreviewOutcome::Ready(ready), None) => self.apply_rehearsed(ready).await,
                    (PreviewOutcome::Deferred, None) => {
                        match self.preview(document.request.clone()).await {
                            Ok(ready) => self.apply_rehearsed(&ready).await,
                            Err(error) => failed(error),
                        }
                    }
                };
            let label = target
                .as_ref()
                .map_or_else(|| document.label.clone(), object_label);
            results.push(DocumentResult {
                index: document.index,
                label,
                target,
                status,
                method,
                message,
                resource_version,
            });
        }
        BatchReport { documents: results }
    }

    /// Sends one rehearsed document and says what it did.
    async fn apply_rehearsed(&self, ready: &ApplyPreview) -> Outcome {
        let change = ready.change();
        match self.apply(ready).await {
            Ok(report) => {
                let status = match change {
                    Change::Create => DocumentStatus::Created,
                    Change::Update => DocumentStatus::Configured,
                    Change::Unchanged => DocumentStatus::Unchanged,
                };
                let resource_version = report
                    .object
                    .pointer("/metadata/resourceVersion")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                (
                    status,
                    report.message,
                    Some(report.target),
                    Some(report.method),
                    resource_version,
                )
            }
            Err(error) => failed(error),
        }
    }
}

fn failed(error: ApplyError) -> Outcome {
    let message = error.to_string();
    (DocumentStatus::Failed(error), message, None, None, None)
}

/// Skipped because `cause` (an earlier result) did not apply.
fn skipped(cause: &DocumentResult) -> Outcome {
    let reason = format!(
        "skipped: it needs {} (document {}), which did not apply",
        cause.label,
        cause.index + 1
    );
    skipped_because(reason)
}

/// Skipped at the dry run, for an earlier document that failed there.
fn skipped_earlier(document: &DocumentPreview) -> Outcome {
    let reason = match document.needs_text() {
        Some(needs) => format!("skipped: it needs {needs}, one of which did not apply"),
        None => "skipped".to_owned(),
    };
    skipped_because(reason)
}

fn skipped_because(reason: String) -> Outcome {
    (
        DocumentStatus::Skipped(reason.clone()),
        reason,
        None,
        None,
        None,
    )
}
