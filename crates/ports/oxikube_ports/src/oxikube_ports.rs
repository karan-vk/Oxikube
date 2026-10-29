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
//! | [`PromqlPort`], [`PromqlEndpointPort`] | [`promql`] | `oxikube_prometheus` |
//! | [`QueryCataloguePort`] (the PromQL of a [`MetricRequest`](oxikube_domain::MetricRequest) per provider, E13-S06) | [`query_catalogue`] | `oxikube_prometheus` |
//! | [`ProviderDetectorPort`] (finds a cluster's Prometheus-compatible service, E13-S05) | [`provider`] | `oxikube_prometheus` |
//! | [`ServiceProxyPort`] (a request to a Service through the API server) | [`service_proxy`] | `oxikube_kube` |
//! | [`DescribePort`] | [`describe`] | `oxikube_describe` |
//! | [`HelmPort`] | [`helm`] | `oxikube_helm` |
//! | [`KustomizePort`] | [`kustomize`] | `oxikube_kustomize` |
//! | [`StatePort`] | [`state`] | `oxikube_state_sqlite` (ADR 0010) |
//! | [`SecretStorePort`] | [`secrets`] | `oxikube_keychain` |
//! | [`NotifierPort`] | [`notifier`] | `oxikube_notify_os` |
//! | [`NotificationSourcePort`] (inbound hooks: port-forward drops, updates, agent permission, E14-S07) | [`notification_source`] | implemented by `oxikube_app::NotificationService` |
//! | [`UpdaterPort`] | [`updater`] | `oxikube_updater` |
//! | [`CrashReporterPort`] | [`crash`] | `oxikube_crash` |
//! | [`FsPort`] | [`fs`] | `oxikube_runtime` (std fs) or the testkit fake |
//! | [`ClockPort`] | [`clock`] | `oxikube_runtime` (system clock) or `FakeClockPort` |
//! | [`EventsPort`] (the cluster's normalised `Event`s as one feed, E14-S01) | [`events`] | `oxikube_kube::KubeEvents` |
//! | [`DrainPort`] (`kubectl drain` over a reader and the guard's writer, E12-S10) | [`drain`] | `oxikube_kube::KubeDrain` |
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
//! | [`FeedStats`], [`FeedStat`], [`FeedVariant`] (watch-budget counters, E04-S13) | [`feed_stats`] |
//! | [`DiscoveryPort`] | [`discovery`] |
//! | [`TableFeedPort`] | [`table`] |
//! | [`LogPort`] | [`log`] |
//! | [`ExecPort`], [`ExecStreamPort`], [`TerminalBackend`] | [`exec`] |
//! | [`PortForwardPort`] | [`portforward`] |
//!
//! # Session ports (E06-S01)
//!
//! The `ClusterSessionManager` in `oxikube_app::session` connects a context through
//! [`ClusterConnectorPort`] and gets the data-plane ports above back as one
//! [`ClusterPorts`] bundle, plus an [`AccessReviewPort`] for the user's capabilities.
//!
//! | Port | Module |
//! |---|---|
//! | [`ClusterConnectorPort`], [`ClusterPorts`], [`HealthReporter`] | [`connector`] |
//! | [`AccessReviewPort`] | [`access`] |
//! | [`WarningPort`], [`ApiWarning`] (the API server's `Warning:` headers, E07-S10) | [`warnings`] |
//! | [`ClusterPrefs`], [`ClusterPrefsTable`], [`WatchBudgetPrefs`] (resolved per-cluster settings, E06-S08; the watch budget, E04-F543) | [`cluster_prefs`] |
//!
//! # Integration and agent ports (E02-S10)
//!
//! Shaped after MCP tool semantics and the ACP client duties, without
//! depending on `rmcp` or `agent-client-protocol` (see each module for the
//! field-by-field mapping).
//!
//! | Port | Module |
//! |---|---|
//! | [`IntegrationPort`] | [`integration`] |
//! | [`ToolPort`], [`ToolDef`] | [`tool`] |
//! | [`ContextProviderPort`], [`Mention`], [`ContentPart`] | [`context`] |
//! | [`AgentPort`], [`AgentClient`] | [`agent`] |

#![deny(missing_docs)]

pub mod access;
pub mod agent;
pub mod clock;
pub mod cloud;
pub mod cluster_prefs;
pub mod cluster_source;
pub mod connector;
pub mod context;
pub mod crash;
pub mod describe;
pub mod discovery;
pub mod drain;
pub mod events;
pub mod exec;
pub mod feed;
pub mod feed_stats;
pub mod fs;
pub mod helm;
pub mod integration;
pub mod kustomize;
pub mod log;
pub mod metrics;
pub mod notification_source;
pub mod notifier;
pub mod portforward;
pub mod promql;
pub mod provider;
pub mod query_catalogue;
pub mod resource;
pub mod schema;
pub mod secrets;
pub mod service_proxy;
pub mod state;
pub mod table;
pub mod tool;
pub mod updater;
pub mod warnings;

pub use access::AccessReviewPort;
pub use agent::{
    AgentCapabilities, AgentClient, AgentInfo, AgentPort, AgentSessionId, AgentUpdateBatch,
    AgentUpdateStream, AuthMethod, AuthMethodId, ClientCapabilities, ClientInfo, CreateTerminal,
    ElicitationMode, ElicitationRequest, ElicitationResponse, McpServerConfig, MessageChunk,
    NewSessionRequest, PermissionOption, PermissionOptionId, PermissionOptionKind,
    PermissionOutcome, PermissionRequest, Plan, PlanEntry, PlanPriority, PlanStatus, ReadTextFile,
    SessionUpdate, StopReason, TerminalExit, TerminalId, TerminalOutput, ToolCallContent,
    ToolCallId, ToolCallInfo, ToolCallPatch, ToolCallStatus, ToolKind, WriteTextFile,
};
pub use clock::ClockPort;
pub use cloud::{
    CloudDetection, CloudDiscoveryPort, CloudProvider, CloudScope, CloudToolStatus, ClusterStatus,
    DiscoveredCluster, ExecEnvVar, ExecInteractiveMode, ExecPlugin, KubeconfigEntry, SetupHint,
};
pub use cluster_prefs::{
    ClusterPrefs, ClusterPrefsTable, MetricsMode, NodeShellPrefs, PrometheusAuthKind,
    PrometheusOverride, WatchBudgetPrefs,
};
pub use cluster_source::{
    ClusterContext, ClusterEndpoint, ClusterSource, ClusterSourcePort, DiagnosticSeverity,
    SourceDiagnostic, SourceId, SourceKind, SourceState, SourceStatus, SourcesChanged, UserSource,
    UserSourceKind,
};
pub use connector::{
    ClusterConnection, ClusterConnectorPort, ClusterPorts, ConnectRequest, ConnectionGuard,
    ExecInteractivity, HealthReporter, HealthSignal,
};
pub use context::{ContentPart, ContextProviderPort, ContextScope, Mention, MentionPrefix};
pub use crash::{CrashId, CrashReport, CrashReporterPort};
pub use describe::{DescribeOutput, DescribePort, DescribeSource};
pub use discovery::{
    CrdWatchStatus, DiscoveryEvent, DiscoveryEvents, DiscoveryPort, KindsChange, ServerVersion,
};
pub use drain::{DrainPort, DrainStream};
pub use events::{
    EventApi, EventApiChoice, EventsFeed, EventsMonitor, EventsPort, EventsStatus,
    EventsWatchOptions, ForbiddenScope,
};
pub use exec::{
    AttachTarget, BackendEvent, DEFAULT_NODE_SHELL_IMAGE, DEFAULT_NODE_SHELL_NAMESPACE,
    DEFAULT_NSENTER_ARGS, DebugContainerSpec, ExecOptions, ExecPort, ExecSession, ExecStreamPort,
    ExecTarget, ExitStatus, HEARTBEAT_ANNOTATION, NODE_ANNOTATION, NODE_SHELL_CONTAINER,
    NODE_SHELL_LABEL, NodeShellSpec, NodeShellToleration, SessionBackend, TerminalBackend,
    TerminalSize, node_shell_command, node_shell_manifest,
};
pub use feed::{Delta, DeltaBatch, WatchFeed};
pub use feed_stats::{FeedStat, FeedStats, FeedVariant};
pub use fs::{DirEntry, EntryKind, FileChunks, FsEvent, FsEventKind, FsPort};
pub use helm::{
    ChartHit, ChartRef, HelmCapabilities, HelmError, HelmOutput, HelmPort, HelmRepoPort,
    HelmVersion, InstallRequest, OutputKind, OutputLine, Release, ReleaseRef, ReleaseStatus,
    RepoAddRequest, RepoEntry, RepoPassword, Revision, RollbackRequest, SearchQuery, ShowKind,
    UninstallRequest, UpgradeRequest, Values,
};
pub use integration::{
    IntegrationPort, IntegrationSession, SidebarItem, SidebarModel, SidebarSection,
};
pub use kustomize::{BuildOutput, KustomizeError, KustomizePort, KustomizeTool, KustomizeToolKind};
pub use log::{LogOptions, LogPort, LogSince, LogStream};
pub use metrics::{MetricsOutcome, MetricsPort};
pub use notification_source::NotificationSourcePort;
pub use notifier::{
    ActivationHandler, MAX_BODY_CHARS, MAX_TITLE_CHARS, Notification, NotificationActivation,
    NotificationLevel, NotifierPort, NotifierStatus, is_unavailable, unavailable,
};
pub use portforward::{
    ActiveConnection, DuplexStream, FIRST_UNPRIVILEGED_PORT, ForwardHandle, ForwardStats,
    ForwardStatsSnapshot, PodRef, PortForwardConnection, PortForwardError, PortForwardPort,
};
pub use promql::{
    DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_TIMEOUT, PromAuth, PromEndpoint, PromSecret, PromTls,
    PromqlEndpointPort, PromqlError, PromqlPort, PromqlSeries, PromqlValue, TimeRange,
};
pub use provider::{
    DetectedProvider, DetectionFailure, DetectionResult, DetectionState, Probe, ProbeOutcome,
    ProviderDetectorPort, ProviderTarget,
};
pub use query_catalogue::{QueryCataloguePort, RangePlan};
pub use resource::{
    DeleteCollectionOutcome, DeleteOptions, DeleteOutcome, ListOptions, ListPage, Patch, PatchKind,
    Preconditions, PropagationPolicy, ResourcePort, ResourceReader, ResourceWriter, Scale,
    Subresource, VersionMatch, WatchOptions, WriteOptions,
};
pub use schema::SchemaPort;
pub use secrets::{SecretKey, SecretStorePort};
pub use service_proxy::{
    ProxyMethod, ProxyRequest, ProxyResponse, ProxyScheme, ServicePort, ServiceProxyPort,
    ServiceRef,
};
pub use state::{AuditQuery, StateKey, StatePort, StatePortExt, StateTable};
pub use table::{
    IncludeObject, Table, TableBatch, TableColumn, TableFeed, TableFeedPort, TableOptions,
    TableRow, TableSource,
};
pub use tool::{ToolAnnotations, ToolContext, ToolDef, ToolName, ToolOutput, ToolPort};
pub use updater::{DownloadedUpdate, UpdateChannel, UpdateInfo, UpdaterPort};
pub use warnings::{ApiWarning, WarningPort};

#[cfg(test)]
mod tests {
    //! Compile-time object-safety checks and the "docs name an adapter" check.

    use std::sync::Arc;

    use super::*;

    /// Each function only has to compile: it proves `Arc<dyn Port>` is valid.
    #[allow(
        dead_code,
        reason = "the fields only have to compile to prove object safety"
    )]
    struct ObjectSafe {
        cluster_source: Arc<dyn ClusterSourcePort>,
        cloud: Arc<dyn CloudDiscoveryPort>,
        metrics: Arc<dyn MetricsPort>,
        promql: Arc<dyn PromqlPort>,
        promql_endpoint: Arc<dyn PromqlEndpointPort>,
        detector: Arc<dyn ProviderDetectorPort>,
        catalogue: Arc<dyn QueryCataloguePort>,
        service_proxy: Arc<dyn ServiceProxyPort>,
        describe: Arc<dyn DescribePort>,
        helm: Arc<dyn HelmPort>,
        kustomize: Arc<dyn KustomizePort>,
        helm_repos: Arc<dyn HelmRepoPort>,
        state: Arc<dyn StatePort>,
        secrets: Arc<dyn SecretStorePort>,
        notifier: Arc<dyn NotifierPort>,
        notification_source: Arc<dyn NotificationSourcePort>,
        updater: Arc<dyn UpdaterPort>,
        crash: Arc<dyn CrashReporterPort>,
        fs: Arc<dyn FsPort>,
        clock: Arc<dyn ClockPort>,
        connector: Arc<dyn ClusterConnectorPort>,
        access: Arc<dyn AccessReviewPort>,
        warnings: Arc<dyn WarningPort>,
        health: Arc<dyn HealthReporter>,
        schema: Arc<dyn SchemaPort>,
        drain: Arc<dyn DrainPort>,
    }

    /// Ports must be shareable across threads: `Arc<dyn Port>` is `Send + Sync`.
    #[test]
    fn ports_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn ClusterSourcePort>();
        assert_send_sync::<dyn CloudDiscoveryPort>();
        assert_send_sync::<dyn MetricsPort>();
        assert_send_sync::<dyn PromqlPort>();
        assert_send_sync::<dyn PromqlEndpointPort>();
        assert_send_sync::<dyn ProviderDetectorPort>();
        assert_send_sync::<dyn QueryCataloguePort>();
        assert_send_sync::<dyn ServiceProxyPort>();
        assert_send_sync::<dyn DescribePort>();
        assert_send_sync::<dyn HelmPort>();
        assert_send_sync::<dyn HelmRepoPort>();
        assert_send_sync::<dyn KustomizePort>();
        assert_send_sync::<dyn StatePort>();
        assert_send_sync::<dyn SecretStorePort>();
        assert_send_sync::<dyn NotifierPort>();
        assert_send_sync::<dyn NotificationSourcePort>();
        assert_send_sync::<dyn UpdaterPort>();
        assert_send_sync::<dyn CrashReporterPort>();
        assert_send_sync::<dyn FsPort>();
        assert_send_sync::<dyn ClockPort>();
        assert_send_sync::<dyn ClusterConnectorPort>();
        assert_send_sync::<dyn AccessReviewPort>();
        assert_send_sync::<dyn SchemaPort>();
        assert_send_sync::<dyn DrainPort>();
        assert_send_sync::<ClusterPorts>();
        assert_send_sync::<ClusterConnection>();
    }

    /// Every port's source file must name the adapter expected to implement it.
    #[test]
    fn every_port_docs_name_an_adapter() {
        let ports: [(&str, &str, &str); 21] = [
            (
                "cluster_source",
                include_str!("cluster_source.rs"),
                "oxikube_kube::sources",
            ),
            ("cloud", include_str!("cloud.rs"), "oxikube_cloud"),
            ("metrics", include_str!("metrics.rs"), "oxikube_kube"),
            ("promql", include_str!("promql.rs"), "oxikube_prometheus"),
            (
                "provider",
                include_str!("provider.rs"),
                "oxikube_prometheus",
            ),
            (
                "query_catalogue",
                include_str!("query_catalogue.rs"),
                "oxikube_prometheus",
            ),
            (
                "service_proxy",
                include_str!("service_proxy.rs"),
                "oxikube_kube",
            ),
            ("describe", include_str!("describe.rs"), "oxikube_describe"),
            ("helm", include_str!("helm.rs"), "oxikube_helm"),
            (
                "kustomize",
                include_str!("kustomize.rs"),
                "oxikube_kustomize",
            ),
            ("state", include_str!("state.rs"), "oxikube_state_sqlite"),
            ("secrets", include_str!("secrets.rs"), "oxikube_keychain"),
            ("notifier", include_str!("notifier.rs"), "oxikube_notify_os"),
            ("updater", include_str!("updater.rs"), "oxikube_updater"),
            ("crash", include_str!("crash.rs"), "oxikube_crash"),
            ("fs", include_str!("fs.rs"), "oxikube_runtime"),
            ("clock", include_str!("clock.rs"), "oxikube_runtime"),
            ("connector", include_str!("connector.rs"), "oxikube_kube"),
            ("access", include_str!("access.rs"), "oxikube_kube"),
            ("schema", include_str!("schema.rs"), "oxikube_kube::openapi"),
            ("drain", include_str!("drain.rs"), "oxikube_kube::KubeDrain"),
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
