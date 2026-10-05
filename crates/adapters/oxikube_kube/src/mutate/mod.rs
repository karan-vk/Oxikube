//! `ResourceWriter` on kube-rs: create, replace, patch, server-side apply, dry-run and delete
//! for any kind (E04-S05).
//!
//! The adapter half of the mutation pipeline (ADR 0012). **Callers reach these methods only
//! through `oxikube_app::mutation::MutationGuard`**; the adapter decides nothing about
//! read-only mode or confirmation tiers, it executes the request and reports the outcome
//! faithfully. Everything runs on `Api<DynamicObject>`, so core kinds and CRDs share one path.
//!
//! | Piece | Where |
//! |---|---|
//! | `ResourceWriter` impl; scale, evict and the subresource methods answer `Unsupported` until E04-S06 | `writer` |
//! | create, replace, patch, delete, delete-collection requests | `ops` |
//! | port options to `PostParams` / `PatchParams` / `DeleteParams`, patch content types | `params` |
//! | error mapping with field managers and field paths | `error` |
//!
//! # Field manager
//!
//! Every write is attributed to a field manager so `managedFields` names Oxikube rather than
//! the HTTP user agent: [`DEFAULT_FIELD_MANAGER`] (`oxikube`) unless
//! [`WriteOptions::field_manager`](oxikube_ports::WriteOptions::field_manager) or, for
//! server-side apply, the patch's own manager says otherwise. Apply sends `force` only when the
//! patch asks for it, so by default a clash with another manager is a conflict the user decides
//! on.
//!
//! # Dry run
//!
//! `dry_run` sets `dryRun=All`. The server runs admission and validation and returns the object
//! as it would be stored (for `delete`, the object as it would be left), changing nothing; the
//! app diffs that against the live object.
//!
//! # Delete collection
//!
//! A namespaced kind needs a namespace: the API server has no cluster-wide `deletecollection`
//! for them, so `namespace: None` is a `Validation` error here (sent, it would be a 405) rather
//! than the "whole cluster" the port's wording allows. Only the label and field selectors of
//! the selection are used.
//!
//! # Errors
//!
//! [`crate::auth::classify`] decides the kind (409 `Conflict`, 422 and 400 `Validation`, 403
//! `Forbidden`, 404 `NotFound`, 415 `Unsupported`, 429 `Network`). On top of that:
//!
//! * a `Conflict` carries [`ConflictDetails`](oxikube_domain::ConflictDetails): for a
//!   server-side apply clash the fields and the field managers that own them, so the UI can
//!   offer "force" or "cancel"; a stale `resourceVersion` and an already-existing object are
//!   told apart too;
//! * a `Validation` carries [`ValidationDetails`](oxikube_domain::ValidationDetails) with the
//!   field paths the server rejected;
//! * a 5xx the taxonomy files under `Internal` (500, 502) is marked retryable, since the
//!   server failed the request rather than the request being wrong.
//!
//! Read the details with `OxiError::conflict_details` and `OxiError::validation_details`.
//! Request bodies are never logged or put in an error: a body may be a Secret.

mod error;
mod ops;
mod params;
#[cfg(test)]
mod tests;
mod writer;

pub use params::DEFAULT_FIELD_MANAGER;
