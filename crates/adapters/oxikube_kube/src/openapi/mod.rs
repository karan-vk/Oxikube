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
//! | the three GETs (index, `/version`, one group document) | `fetch` |
//! | disk cache (raw group documents keyed by cluster + server version + index hash) | `cache` |
//! | [`OpenApiConfig`]: the request and miss deadlines | `config` |
//! | [`OpenApiSchemas`]: `SchemaPort` impl, single-flight, memory cache | `service` |
//! | re-checking the index just after an `invalidate`, for a server that lags | `settle` |
//!
//! # Caching
//!
//! The full v3 document set is tens of MB on large clusters, so nothing is
//! fetched eagerly: the index (and `/version`) load on the first `schema_for`,
//! each group document on the first GVK inside it. Parsed schemas live in memory
//! keyed by GVK. Raw group documents live on disk under
//! `<cluster>/<server version>/<group-version>-<index hash>.json`: the hash is
//! the server's content hash of the document, so a restart re-reads the server
//! only when the API changed (an upgrade or a CRD edit changes the name; stale
//! files are removed on the next write; files for other server versions are
//! pruned once per run, unless `/version` could not be read). A server without a
//! hash is never cached on disk; a document that cannot be kept on disk (no disk
//! cache, a failed write, no hash) is held in memory instead, so every further kind
//! of its group parses it without another download.
//! [`SchemaPort::invalidate`](oxikube_ports::SchemaPort::invalidate) (called on
//! discovery's `KindsChanged` or `SchemasChanged`, the latter for an in-place CRD schema edit) drops the memory state, so the next lookup reads
//! the fresh index and re-fetches only the documents whose hash changed. A
//! lookup that finds nothing also re-reads an index older than
//! [`OpenApiConfig::refresh_on_miss_after`], since a new CRD reaches the OpenAPI
//! document a moment after discovery reports it. The same lag can hit an `invalidate`: a
//! CRD edit reaches the aggregated index a fraction of a second after the CRD watch reports
//! it, so the first lookup may read the old index and cache the old schema as a plain hit.
//! For [`OpenApiConfig::settle_after_invalidate`] after an invalidate, lookups therefore
//! re-read the index (at most once per [`OpenApiConfig::recheck_every`]) and drop what
//! changed, until one read has been made after that settling time. For that same window a kind
//! with no schema, and a server without `/openapi/v3` (`Unsupported`), answer
//! without a request, so a validator asking on every edit costs nothing.
//!
//! # Concurrency
//!
//! Memory hits take only a short lock. The index load is single-flight (`/version`
//! runs beside it but never delays an index error), and so is each group-version
//! document: two editors opening the same group at once
//! trigger one HTTP fetch, while different groups fetch in parallel. A fetch
//! that was in flight during `invalidate` drops its result. Parsing and
//! flattening run on the blocking pool, never on an async worker or the UI
//! thread.

mod cache;
mod config;
mod fetch;
mod index;
mod service;
mod settle;

#[cfg(test)]
mod tests;

pub use config::{
    DEFAULT_RECHECK_EVERY_MILLIS, DEFAULT_REFRESH_ON_MISS_SECS, DEFAULT_REQUEST_TIMEOUT_SECS,
    DEFAULT_SETTLE_AFTER_INVALIDATE_MILLIS, OpenApiConfig,
};
pub use service::OpenApiSchemas;
