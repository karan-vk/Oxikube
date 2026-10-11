//! Many documents in one apply (E10-S09): the buffer of a new resource, or any pasted
//! multi-document manifest, applied in order.
//!
//! ```text
//! documents ─preview_batch─> every document's dry run (eight at a time, through the guard)
//!                              ├─ the namespace or CRD an earlier document creates is missing: deferred
//!                              ├─ an earlier document it needs failed: skipped
//!                              └─ the rest: previewed, or failed with the server's message
//! user confirms the combined preview
//! BatchPreview ─apply_batch─> in buffer order, one resource::Apply each
//!                              ├─ a failure does not stop the rest (as `kubectl apply`)
//!                              └─ except the documents that need the failed one: skipped
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `types` | [`BatchPreview`], [`DocumentPreview`], [`PreviewOutcome`], [`BatchReport`], [`DocumentResult`], [`DocumentStatus`] |
//! | `depends` | which earlier document a later one needs: its Namespace, its CustomResourceDefinition |
//! | `service` | `ApplyService::preview_batch` / `apply_batch` |
//!
//! There is no second apply path: every document is the same `resource::Apply` dispatch as a
//! single object, so each is guarded (a read-only cluster refuses all of them, the dry-run
//! preview is the confirmation, each is audited with its initiator) and server-side applied
//! under the same field manager with the same replace / create fallback.

mod depends;
mod service;
mod types;

pub use types::{
    BatchCounts, BatchPreview, BatchReport, DocumentPreview, DocumentResult, DocumentStatus,
    PreviewOutcome,
};
