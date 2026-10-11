//! A buffer of several documents (E10-S09): every document is dry-run, one combined preview is
//! confirmed, then they are applied in buffer order and each one's result is shown.
//!
//! ```text
//! cmd-s on a buffer of 2+ documents ─> the buffer's own checks (errors refuse)
//!   ─> ApplyService::preview_batch (every document's server dry run, on the Kubernetes runtime)
//!   ─> BatchPreviewDialog: the documents with what each would do, the selected one's diff
//!        └─ Apply ─> ApplyService::apply_batch ─> ResultsPanel under the buffer
//!                      created / configured / unchanged / failed (message) / skipped,
//!                      a click on a row selects its document, a rejected field gets a squiggle
//! ```
//!
//! | Module | What |
//! |---|---|
//! | `flow` | the steps above on [`ManifestEditor`](crate::view::ManifestEditor): the preview off the UI thread, the dialog, the apply, the server's causes put on their documents |
//! | `dialog` | [`BatchPreviewDialog`]: the combined preview; `dialog::rows`: the virtualised list of documents and the selected diff |
//! | `results` | the per-document results panel in the editor's bottom area (virtualised) |
//!
//! The documents are the buffer's own (`crate::apply::split_documents`, the spanned YAML model),
//! each sent as `resource::Apply` with the same field manager: one guard, one audit record per
//! document, the same fallback. The policy is `kubectl apply`'s: a failure does not stop the
//! documents after it, except those that need what failed (a namespace or a definition an earlier
//! document makes), which are skipped; the panel says so. Nothing here logs a document.

mod dialog;
mod flow;
mod results;

use std::ops::Range;
use std::sync::Arc;

pub use dialog::BatchPreviewDialog;
pub(super) use flow::{Target, run_preview};
pub(in crate::view) use results::ResultsPanel;

/// The buffer a batch was made from: its text (to put the server's causes on their documents),
/// where each document is, and the version it was at.
#[derive(Clone, Debug)]
pub struct BatchSession {
    /// The buffer's text.
    pub text: Arc<str>,
    /// Where each document is in it ([`split_documents`](crate::apply::split_documents)).
    pub ranges: Vec<Range<usize>>,
    /// The buffer version it was made at: a result for another version is stale.
    pub version: u64,
}
