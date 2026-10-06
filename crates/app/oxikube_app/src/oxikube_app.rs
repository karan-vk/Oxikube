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
//! | [`sidebar`] | E06-S10 | [`review_access`](sidebar::review_access) (the rules reviews the cluster sidebar hides sections by, failing open) and [`discover_custom_resources`](sidebar::discover_custom_resources) |
//! | [`integrations`] | E06-S10 | [`IntegrationRegistry`] (stub): registered integrations and the sidebar sections they append after the core ones |
//! | [`actions`] | E07-S08 | row actions: [`RowActionRegistry`] / [`RowActions`] (the actions a table's context menu and the palette offer per kind, read from the `CommandBus` once), the `resource::Delete` handler, and [`DeleteFlow`] (plan and run a delete of one object or a selection through the guard, with per-object results) |
//! | [`audit`] | E06-S02 | [`AuditLog`]: redacted, batched, fail-closed audit appends through `StatePort` |
//! | [`store`] | E07-S01 | [`ResourceStore`]: the per-session cache over reflector, metadata and Table feeds keyed by (gvk, scope), ref-counted feeds with grace teardown and a [`FeedBudget`](store::FeedBudget) hook, in-app sort / filter / name-namespace-label indices, and [`Subscription`](store::Subscription) streams of coalesced [`StoreDelta`](store::StoreDelta)s; [`ResourceStores`] keeps one per connected session |
//! | [`columns`] | E07-S02 | [`ColumnProvider`]: table columns and cells per kind. [`CoreColumns`] (a table-driven catalogue of ~40 core kinds; Ready / Status / Restarts from the domain view-models; CPU and memory as pending metrics hooks) and [`TableColumns`] (a Table feed's server columns, `priority > 0` as `wide`); [`Cell`]s carry text, a typed sort key and a tone |
//! | [`session::namespaces`] | E06-S07 | [`NamespaceService`](session::namespaces::NamespaceService): namespace selection remembered per cluster, favourites, the namespace list with the RBAC fallback, `namespace::*` commands |

pub mod actions;
pub mod audit;
pub mod catalog;
pub mod columns;
pub mod command_bus;
pub mod guard;
pub mod integrations;
pub mod session;
pub mod sidebar;
pub mod sources;
pub mod store;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod testing_posture;

pub use actions::{
    ActionContext, ActionState, DeleteError, DeleteFlow, DeletePlan, DeleteReport, ItemResult,
    ItemStatus, ResolvedAction, RowAction, RowActionRegistry, RowActionSpec, RowActions,
    object_label,
};
pub use audit::AuditLog;
pub use catalog::{
    CatalogEntry, ClusterCatalog, ClusterCommandOutcome, ClusterCommands, FavouriteChanged,
    FavouritesLagged,
};
pub use columns::{Cell, Column, ColumnId, ColumnProvider, CoreColumns, TableColumns};
pub use command_bus::{
    CommandBus, CommandHandler, CommandOutput, CommandRegistry, DispatchContext, DispatchError,
    HandlerContext, Outcome, RegisterError,
};
pub use guard::{
    Confirmation, ConfirmationRequest, ConfirmationToken, Mutation, MutationGuard, PrefsPatch,
    PrefsWriter,
};
pub use integrations::{IntegrationRegistry, IntegrationSection, RegisterIntegrationError};
pub use session::{
    ClusterSession, ClusterSessionManager, SessionChange, SessionUpdate, SessionUpdates,
};
pub use sidebar::{AccessOutcome, CustomKind, CustomResourceGroup};
pub use sources::{KubeconfigSourcesService, SourceListStore, SourceRow};
pub use store::{
    CountState, CountTarget, CountsLease, KindCount, ResourceStore, ResourceStores, StoreDelta,
    StoreQuery, Subscription,
};
