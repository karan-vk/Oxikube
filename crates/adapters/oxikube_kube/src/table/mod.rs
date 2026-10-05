//! `TableFeedPort` on kube-rs: the hand-rolled server Table API feed (E04-S04, ADR 0006).
//!
//! kube-rs has no Table support, yet the Table API is the only way to show any kind the way
//! `kubectl get` does, CRD `additionalPrinterColumns` included. This module builds the
//! requests itself over `Client::request_text` / `request_events` (prior art: kubetui,
//! sofka; research 2.3a) and hands upward the port's `Table` / `TableBatch`, never a kube
//! type (ADR 0005). It is implemented on [`KubeResources`](crate::KubeResources), sharing
//! discovery, `ListOptions` handling and error mapping with the resource reads.
//!
//! | Piece | Where |
//! |---|---|
//! | `Accept: application/json;as=Table;v=v1;g=meta.k8s.io,application/json` + `includeObject` | `request` |
//! | own `Table` / `TableRow` / `ColumnDefinition` serde types | `wire` |
//! | wire to port types; the plain-JSON fallback columns | `convert` |
//! | collection resolution, one page, `list_table` | `list` |
//! | row identity and refresh diffing | `index` |
//! | list + watch loop, polling, backpressure | `feed` |
//! | settings (refresh interval, timeouts, page size, batch size) | [`TableConfig`] |
//!
//! # Fallback
//!
//! The `Accept` list ends with plain `application/json`, so a server that cannot build a
//! Table (some aggregated APIs) answers with the ordinary list. That is detected by
//! `kind != "Table"`: rows then carry the apiserver's generic `Name` / `Created At` columns
//! and every `Table` / `TableBatch` says [`TableSource::Objects`](oxikube_ports::TableSource)
//! so `ColumnProvider` can substitute its own (ADR 0006).
//!
//! # Rows and cells
//!
//! Rows are keyed by the UID from `includeObject=Metadata` and diffed by `resourceVersion`.
//! Server-rendered cells such as `Age` (`"15h"`) are a snapshot: they change without the
//! object changing, so they are not re-sent; render ages from `TableRow::meta.creation`.

mod config;
mod convert;
mod feed;
mod index;
mod list;
mod port;
mod request;
#[cfg(test)]
mod tests;
mod wire;

pub use config::TableConfig;
pub use feed::CHANNEL_CAPACITY;
pub use request::TABLE_ACCEPT;
