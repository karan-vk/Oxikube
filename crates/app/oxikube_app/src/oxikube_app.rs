//! `oxikube_app` — layer: `app`.
//!
//! Use-cases and services (no gpui, no kube): ClusterSessionManager, ResourceStore, CommandBus, MutationGuard + AuditService, LogService, ExecService, PortForwardManager, EventService, NotificationService, MetricsService, HelmService, DescribeService, SearchService, IntegrationRegistry, ToolRegistry, ContextRegistry, AgentSessionManager, ApplyService.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! # Modules
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`session`] | E06-S01 | [`ClusterSessionManager`]: connect / disconnect / reconnect, the session state machine, capabilities, namespace selection, read-only flag, colour, and the [`SessionUpdates`] stream |

pub mod session;

pub use session::{
    ClusterSession, ClusterSessionManager, SessionChange, SessionUpdate, SessionUpdates,
};
