//! `oxikube_ports` — layer: `ports`.
//!
//! Async, object-safe port traits the app depends on and adapters implement.
//! This crate holds the **infrastructure ports** (E02-S09), one module per port:
//!
//! | Port | Module | Expected adapter |
//! |---|---|---|
//! | [`ClusterSourcePort`] | [`cluster_source`] | `oxikube_kube::sources` |
//! | [`CloudDiscoveryPort`] | [`cloud`] | `oxikube_cloud` |
//! | [`MetricsPort`] | [`metrics`] | `oxikube_kube` (metrics.k8s.io, ADR 0011) |
//! | [`PromqlPort`] | [`promql`] | `oxikube_prometheus` |
//! | [`DescribePort`] | [`describe`] | `oxikube_describe` |
//! | [`HelmPort`] | [`helm`] | `oxikube_helm` |
//! | [`StatePort`] | [`state`] | `oxikube_state_sqlite` (ADR 0010) |
//! | [`SecretStorePort`] | [`secrets`] | `oxikube_keychain` |
//! | [`NotifierPort`] | [`notifier`] | `oxikube_notify_os` |
//! | [`UpdaterPort`] | [`updater`] | `oxikube_updater` |
//! | [`CrashReporterPort`] | [`crash`] | `oxikube_crash` |
//! | [`FsPort`] | [`fs`] | `oxikube_runtime` (std fs) or the testkit fake |
//! | [`ClockPort`] | [`clock`] | `oxikube_runtime` (system clock) or `FakeClockPort` |
//!
//! Every trait is declared with `#[async_trait]` and is object-safe, so the app
//! holds them as `Arc<dyn Port>`. Every fallible method returns
//! [`OxiResult`](oxikube_domain::OxiResult). Streams are
//! [`BoxStream`](futures::stream::BoxStream)s so no async runtime leaks into
//! this crate. Mutating methods are marked **Mutating** in rustdoc; they are
//! reachable only through `MutationGuard` (non-negotiable 3).
//!
//! The data-plane ports (E02-S08) live beside them; see the table below.
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

pub mod clock;
pub mod cloud;
pub mod cluster_source;
pub mod crash;
pub mod describe;
pub mod discovery;
pub mod exec;
pub mod feed;
pub mod fs;
pub mod helm;
pub mod log;
pub mod metrics;
pub mod notifier;
pub mod portforward;
pub mod promql;
pub mod resource;
pub mod secrets;
pub mod state;
pub mod table;
pub mod updater;

pub use clock::ClockPort;
pub use cloud::{CloudDiscoveryPort, CloudProvider, CloudToolStatus, DiscoveredCluster};
pub use cluster_source::{
    ClusterContext, ClusterSource, ClusterSourcePort, SourceId, SourceKind, SourcesChanged,
};
pub use crash::{CrashId, CrashReport, CrashReporterPort};
pub use describe::{DescribeOutput, DescribePort, DescribeSource};
pub use discovery::{DiscoveryPort, ServerVersion};
pub use exec::{ExecOptions, ExecPort, ExecSession, ExitStatus, TerminalSize};
pub use feed::{Delta, DeltaBatch, WatchFeed};
pub use fs::{DirEntry, EntryKind, FsEvent, FsEventKind, FsPort};
pub use helm::{HelmPort, HelmRelease, HelmReleaseRef, HelmReleaseStatus};
pub use log::{LogOptions, LogPort, LogSince, LogStream};
pub use metrics::{MetricsOutcome, MetricsPort};
pub use notifier::{Notification, NotificationLevel, NotifierPort};
pub use portforward::{DuplexStream, PortForwardConnection, PortForwardPort};
pub use promql::{PromqlPort, PromqlSeries, PromqlValue, TimeRange};
pub use resource::{
    DeleteCollectionOutcome, DeleteOptions, DeleteOutcome, ListOptions, ListPage, Patch, PatchKind,
    Preconditions, PropagationPolicy, ResourcePort, ResourceReader, ResourceWriter, Scale,
    Subresource, VersionMatch, WatchOptions, WriteOptions,
};
pub use secrets::{SecretKey, SecretStorePort};
pub use state::{AuditQuery, StateKey, StatePort, StatePortExt, StateTable};
pub use table::{
    IncludeObject, Table, TableBatch, TableColumn, TableFeed, TableFeedPort, TableOptions, TableRow,
};
pub use updater::{DownloadedUpdate, UpdateChannel, UpdateInfo, UpdaterPort};

#[cfg(test)]
mod tests {
    //! Compile-time object-safety checks and the "docs name an adapter" check.

    use std::sync::Arc;

    use super::*;

    /// Each function only has to compile: it proves `Arc<dyn Port>` is valid.
    #[allow(dead_code)]
    struct ObjectSafe {
        cluster_source: Arc<dyn ClusterSourcePort>,
        cloud: Arc<dyn CloudDiscoveryPort>,
        metrics: Arc<dyn MetricsPort>,
        promql: Arc<dyn PromqlPort>,
        describe: Arc<dyn DescribePort>,
        helm: Arc<dyn HelmPort>,
        state: Arc<dyn StatePort>,
        secrets: Arc<dyn SecretStorePort>,
        notifier: Arc<dyn NotifierPort>,
        updater: Arc<dyn UpdaterPort>,
        crash: Arc<dyn CrashReporterPort>,
        fs: Arc<dyn FsPort>,
        clock: Arc<dyn ClockPort>,
    }

    /// Ports must be shareable across threads: `Arc<dyn Port>` is `Send + Sync`.
    #[test]
    fn ports_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn ClusterSourcePort>();
        assert_send_sync::<dyn CloudDiscoveryPort>();
        assert_send_sync::<dyn MetricsPort>();
        assert_send_sync::<dyn PromqlPort>();
        assert_send_sync::<dyn DescribePort>();
        assert_send_sync::<dyn HelmPort>();
        assert_send_sync::<dyn StatePort>();
        assert_send_sync::<dyn SecretStorePort>();
        assert_send_sync::<dyn NotifierPort>();
        assert_send_sync::<dyn UpdaterPort>();
        assert_send_sync::<dyn CrashReporterPort>();
        assert_send_sync::<dyn FsPort>();
        assert_send_sync::<dyn ClockPort>();
    }

    /// Every port's source file must name the adapter expected to implement it.
    #[test]
    fn every_port_docs_name_an_adapter() {
        let ports: [(&str, &str, &str); 13] = [
            (
                "cluster_source",
                include_str!("cluster_source.rs"),
                "oxikube_kube::sources",
            ),
            ("cloud", include_str!("cloud.rs"), "oxikube_cloud"),
            ("metrics", include_str!("metrics.rs"), "oxikube_kube"),
            ("promql", include_str!("promql.rs"), "oxikube_prometheus"),
            ("describe", include_str!("describe.rs"), "oxikube_describe"),
            ("helm", include_str!("helm.rs"), "oxikube_helm"),
            ("state", include_str!("state.rs"), "oxikube_state_sqlite"),
            ("secrets", include_str!("secrets.rs"), "oxikube_keychain"),
            ("notifier", include_str!("notifier.rs"), "oxikube_notify_os"),
            ("updater", include_str!("updater.rs"), "oxikube_updater"),
            ("crash", include_str!("crash.rs"), "oxikube_crash"),
            ("fs", include_str!("fs.rs"), "oxikube_runtime"),
            ("clock", include_str!("clock.rs"), "oxikube_runtime"),
        ];
        for (module, source, adapter) in ports {
            let docs: String = source
                .lines()
                .filter(|l| l.trim_start().starts_with("//!") || l.trim_start().starts_with("///"))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                docs.contains(adapter),
                "module `{module}` docs must name its adapter `{adapter}`"
            );
        }
    }
}
