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
//! | [`session`] `prefs` | E06-S08 | per-cluster settings: `set_prefs_table` pushes the resolved `clusters.<id>` values; new sessions start from them, open ones follow them live |
//! | [`command_bus`] | E06-S02 | [`CommandBus`]: dispatch by command id, the per-crate [`CommandRegistry`], MCP tool stubs |
//! | [`guard`] | E06-S02 | [`MutationGuard`]: read-only check, confirmation tier and token, dry-run stage (stub), the [`Mutation`] permit, audit |
//! | [`audit`] | E06-S02 | [`AuditLog`]: redacted, batched, fail-closed audit appends through `StatePort` |
//! | [`session::namespaces`] | E06-S07 | [`NamespaceService`](session::namespaces::NamespaceService): namespace selection remembered per cluster, favourites, the namespace list with the RBAC fallback, `namespace::*` commands |

pub mod audit;
pub mod command_bus;
pub mod guard;
pub mod session;

#[cfg(test)]
mod testing;

pub use audit::AuditLog;
pub use command_bus::{
    CommandBus, CommandHandler, CommandOutput, CommandRegistry, DispatchContext, DispatchError,
    HandlerContext, Outcome, RegisterError,
};
pub use guard::{Confirmation, ConfirmationRequest, ConfirmationToken, Mutation, MutationGuard};
pub use session::{
    ClusterSession, ClusterSessionManager, SessionChange, SessionUpdate, SessionUpdates,
};
