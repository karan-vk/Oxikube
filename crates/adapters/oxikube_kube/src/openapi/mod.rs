//! `SchemaPort` on kube-rs: per-GVK JSON Schemas from OpenAPI v3 (E10-S01).
//!
//! kube-rs has no OpenAPI v3 client, so this module issues the requests itself
//! over `Client::request_text`: `GET /openapi/v3` once for the index of
//! group-version documents, then one lazy `GET` per group-version path on first
//! use. `$ref` and `allOf` flattening is the pure domain code
//! ([`root_schema_for`](oxikube_domain::schema::root_schema_for)); this module
//! owns fetching, single-flight, the in-memory cache and the disk cache.
//!
//! | Piece | Where |
//! |---|---|
//! | index parsing (`paths` → group-version → URL with `?hash=`) | `index` |
//! | disk cache (raw group documents keyed by cluster + index hash) | `cache` |
//! | [`OpenApiSchemas`]: `SchemaPort` impl, single-flight, memory cache | `service` |
//!
//! # Caching
//!
//! The full v3 document set is tens of MB on large clusters, so nothing is
//! fetched eagerly: the index loads on the first `schema_for`, each group
//! document on the first GVK inside it. Parsed schemas live in memory keyed by
//! (cluster, GVK); raw group documents live on disk keyed by cluster and the
//! index entry's hash, so a restart re-reads the server only when the API
//! changed. [`SchemaPort::invalidate`](oxikube_ports::SchemaPort::invalidate)
//! (on discovery's `KindsChanged`) drops both.
//!
//! # Concurrency
//!
//! One async mutex guards the whole fetch path: two editors opening the same
//! group at once trigger one HTTP fetch, and the second caller reads the
//! memory cache. Schema fetches are rare, so serialising them is fine.

mod cache;
mod index;
mod service;

#[cfg(test)]
mod tests;

pub use service::{DEFAULT_REQUEST_TIMEOUT_SECS, OpenApiConfig, OpenApiSchemas};
