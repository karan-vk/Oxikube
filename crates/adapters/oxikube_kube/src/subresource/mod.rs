//! Subresources and the small patches behind scale, restart, cordon and suspend (E04-S06).
//!
//! The subresource half of `ResourceReader` / `ResourceWriter` on [`KubeResources`](crate::KubeResources): `scale`
//! (typed, as a [`Scale`](oxikube_ports::Scale)), `status`, `ephemeralcontainers`, `resize`,
//! `eviction` and any other subresource as raw JSON. **Mutating calls are reachable only
//! through `oxikube_app::mutation::MutationGuard`** (ADR 0012); this module executes the
//! request and reports the outcome, it decides nothing about confirmation or read-only mode.
//!
//! | Piece | Where |
//! |---|---|
//! | raw `get` / `create` / `patch` / `replace` of a named subresource | `request` |
//! | `get_scale`, `scale` (patch of the `scale` subresource, never `spec.replicas`) | `scale` |
//! | `evict` (a `policy/v1` `Eviction`) and the PDB-blocked marker ([`EvictionBlocked`]) | `evict` |
//! | error mapping (missing subresource, eviction refused by a budget) | `error` |
//! | [`ResourcePatch`]: rollout restart, cordon, uncordon, cronjob suspend (ported from kdash) | `patches` |
//! | [`ephemeral_container_patch`], [`resize_patch`]: pod subresource bodies | `pod_patches` |
//!
//! # Why requests are built here
//!
//! kube's `Api<K>` offers `get_scale`, `get_status`, `get_resize` and friends, but several are
//! gated on marker traits implemented only for the bundled `Pod` type, and `Api::evict` sends
//! `delete_options` (snake case), which the server silently drops. Everything here runs on
//! the dynamic path instead: one request builder, bodies and responses as plain JSON, the same
//! code for core kinds and CRDs. Request and response bodies are never logged or put in an
//! error (a subresource body can carry a token).
//!
//! # Errors
//!
//! As for writes (`mutate`): 409 `Conflict` with details, 422 `Validation` with field paths,
//! 403 `Forbidden`, 404 `NotFound`. On top of that:
//!
//! * a 404 that names no object means the server does not serve that subresource for the kind
//!   (a CRD without `scale`, a cluster older than `resize`); it is `Unsupported`, so the UI can
//!   hide the action instead of reporting a missing object;
//! * an eviction the server refuses with HTTP 429 because a PodDisruptionBudget forbids it is
//!   `Network` and retryable (the port's contract), carrying an [`EvictionBlocked`] marker with
//!   the budget's explanation. Read it with [`eviction_blocked`]; a drain (E04-S07) retries
//!   on it. A 429 from API priority and fairness has no marker. The pool's default client
//!   retries 429 itself (backoff, `Retry-After`, up to 15 times), which would hold the
//!   refusal back for minutes: build the adapter with
//!   [`KubeResources::with_unretried_client`](crate::KubeResources::with_unretried_client),
//!   a client from a pool configured with `RetryMode::Disabled`, and evictions use it.
//!
//! # Dry run
//!
//! `WriteOptions::dry_run` sets `dryRun=All` on patch, replace and create; for `evict` it is
//! also set in the `Eviction`'s delete options.

mod error;
mod evict;
mod patches;
mod pod_patches;
mod request;
mod scale;
#[cfg(test)]
mod tests;

pub use error::{EvictionBlocked, eviction_blocked};
pub use patches::{RESTARTED_AT_ANNOTATION, ResourcePatch};
pub use pod_patches::{
    EphemeralContainerSpec, ResizeSpec, ephemeral_container_patch, resize_patch,
};
pub(crate) use request::segment;
