//! `oxikube_ports` — layer: `ports`.
//!
//! Async, object-safe port traits the app depends on and adapters implement: ClusterSourcePort, CloudDiscoveryPort, ResourcePort, TableFeedPort, DiscoveryPort, LogPort, ExecPort, PortForwardPort, MetricsPort, PromqlPort, DescribePort, HelmPort, StatePort, SecretStorePort, NotifierPort, UpdaterPort, CrashReporterPort, IntegrationPort, ToolPort, ContextProviderPort, AgentPort, FsPort, ClockPort, SchemaPort.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! # Conventions
//!
//! * Every port is an `#[async_trait]` trait with `Send + Sync` as supertraits,
//!   so `Arc<dyn Port>` is shareable across tasks. `tests/object_safety.rs`
//!   proves each one is object-safe.
//! * Every fallible method returns [`OxiResult`](oxikube_domain::OxiResult).
//! * Signatures use domain types, `serde_json`, `jiff` and `futures` only;
//!   never `kube`, `k8s-openapi`, `tokio` or `gpui` types.
//! * Mutating methods are documented "Mutating: call only through
//!   `MutationGuard`" and live on traits the guard can own
//!   ([`ResourceWriter`]).
//!
//! # Data-plane ports (E02-S08)
//!
//! | Port | Module |
//! |---|---|
//! | [`ResourcePort`] = [`ResourceReader`] + [`ResourceWriter`] | [`resource`] |
//! | [`WatchFeed`], [`Delta`], [`DeltaBatch`] | [`feed`] |
//! | [`DiscoveryPort`] | [`discovery`] |
//! | [`TableFeedPort`] | [`table`] |
//! | [`LogPort`] | [`log`] |
//! | [`ExecPort`] | [`exec`] |
//! | [`PortForwardPort`] | [`portforward`] |

#![deny(missing_docs)]

pub mod discovery;
pub mod exec;
pub mod feed;
pub mod log;
pub mod portforward;
pub mod resource;
pub mod table;

pub use discovery::{DiscoveryPort, ServerVersion};
pub use exec::{ExecOptions, ExecPort, ExecSession, ExitStatus, TerminalSize};
pub use feed::{Delta, DeltaBatch, WatchFeed};
pub use log::{LogOptions, LogPort, LogSince, LogStream};
pub use portforward::{DuplexStream, PortForwardConnection, PortForwardPort};
pub use resource::{
    DeleteCollectionOutcome, DeleteOptions, DeleteOutcome, ListOptions, ListPage, Patch, PatchKind,
    Preconditions, PropagationPolicy, ResourcePort, ResourceReader, ResourceWriter, Scale,
    Subresource, VersionMatch, WatchOptions, WriteOptions,
};
pub use table::{
    IncludeObject, Table, TableBatch, TableColumn, TableFeed, TableFeedPort, TableOptions, TableRow,
};
