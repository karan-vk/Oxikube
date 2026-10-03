//! `oxikube_ports` — layer: `ports`.
//!
//! Async, object-safe port traits the app depends on and adapters implement: ClusterSourcePort, CloudDiscoveryPort, ResourcePort, TableFeedPort, DiscoveryPort, LogPort, ExecPort, PortForwardPort, MetricsPort, PromqlPort, DescribePort, HelmPort, StatePort, SecretStorePort, NotifierPort, UpdaterPort, CrashReporterPort, IntegrationPort, ToolPort, ContextProviderPort, AgentPort, FsPort, ClockPort, SchemaPort.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
