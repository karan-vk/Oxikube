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
//! | [`catalog`] | E06-S03 | [`ClusterCatalog`]: the kubeconfig contexts with favourites and last-used, and [`ClusterCommands`], the handler of `cluster::Connect`, `cluster::Disconnect` and `cluster::ToggleFavourite` |
//! | [`session`] | E06-S01 | [`ClusterSessionManager`]: connect / disconnect / reconnect, the session state machine, capabilities, namespace selection, read-only flag, colour, and the [`SessionUpdates`] stream |
//! | [`session`] `prefs` | E06-S08 | per-cluster settings: `set_prefs_table` pushes the resolved `clusters.<id>` values; new sessions start from them, open ones follow them live |
//! | [`sources`] | E06-S05 | [`KubeconfigSourcesService`]: the user's kubeconfig sources (add a file or folder, paste, remove, reload) over the settings list, the `ClusterSourcePort` and `FsPort`, and the `kubeconfig::*` command handlers |
//! | [`command_bus`] | E06-S02 | [`CommandBus`]: dispatch by command id, the per-crate [`CommandRegistry`], MCP tool stubs |
//! | [`guard`] | E06-S02 | [`MutationGuard`]: read-only check, confirmation tier and token, dry-run stage (stub), the [`Mutation`] permit, audit |
//! | [`guard::posture`] | E06-S09 | read-only mode, colour and presets as commands (`cluster::ToggleReadOnly`, `cluster::SetColour`, `cluster::ApplyPreset`): confirm when lifting read-only on a production-flagged cluster, audit, the [`PrefsWriter`] port to the settings; the guard also re-checks the flag right before each request |
//! | [`audit`] | E06-S02 | [`AuditLog`]: redacted, batched, fail-closed audit appends through `StatePort` |
//! | [`session::namespaces`] | E06-S07 | [`NamespaceService`](session::namespaces::NamespaceService): namespace selection remembered per cluster, favourites, the namespace list with the RBAC fallback, `namespace::*` commands |

pub mod audit;
pub mod catalog;
pub mod command_bus;
pub mod guard;
pub mod session;
pub mod sources;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod testing_posture;

pub use audit::AuditLog;
pub use catalog::{
    CatalogEntry, ClusterCatalog, ClusterCommandOutcome, ClusterCommands, FavouriteChanged,
    FavouritesLagged,
};
pub use command_bus::{
    CommandBus, CommandHandler, CommandOutput, CommandRegistry, DispatchContext, DispatchError,
    HandlerContext, Outcome, RegisterError,
};
pub use guard::{
    Confirmation, ConfirmationRequest, ConfirmationToken, Mutation, MutationGuard, PrefsPatch,
    PrefsWriter,
};
pub use session::{
    ClusterSession, ClusterSessionManager, SessionChange, SessionUpdate, SessionUpdates,
};
pub use sources::{KubeconfigSourcesService, SourceListStore, SourceRow};
