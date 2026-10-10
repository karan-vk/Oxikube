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
//! | [`search`] | E11-S04, E11-S11 | [`search::fuzzy`]: [`FuzzyService`], the one fuzzy ranking engine; [`search::recents`]: [`StateRecents`] and [`JumpHistory`], persisted through `StatePort`; [`search::aliases`]: [`AliasTable`], the words the `:` jump bar understands (built-in k9s aliases, aliases derived from API discovery, the user's `aliases.json`, collisions listed, never silent), and [`AliasRegistry`], one table per cluster kept in step with the sessions' discovery |
//! | [`session`] | E06-S01 | [`ClusterSessionManager`]: connect / disconnect / reconnect, the session state machine, capabilities, namespace selection, read-only flag, colour, and the [`SessionUpdates`] stream |
//! | [`session`] `prefs` | E06-S08 | per-cluster settings: `set_prefs_table` pushes the resolved `clusters.<id>` values; new sessions start from them, open ones follow them live |
//! | [`sources`] | E06-S05 | [`KubeconfigSourcesService`]: the user's kubeconfig sources (add a file or folder, paste, remove, reload) over the settings list, the `ClusterSourcePort` and `FsPort`, and the `kubeconfig::*` command handlers |
//! | [`command_bus`] | E06-S02 | [`CommandBus`]: dispatch by command id, the per-crate [`CommandRegistry`], MCP tool stubs |
//! | [`command_bus`] `availability`, `index` | E11-S01 | introspection for the palette, help overlay and keymap: [`CommandBus::list`] (the commands runnable in a [`CommandContext`]), `all`, `get`; [`CommandInfo`] / [`CommandIndex`] (sorted once, category then title), [`Selection`] and [`Unavailable`] |
//! | [`guard`] | E06-S02 | [`MutationGuard`]: read-only check, confirmation tier and token, dry-run stage (stub), the [`Mutation`] permit, audit |
//! | [`guard::posture`] | E06-S09 | read-only mode, colour and presets as commands (`cluster::ToggleReadOnly`, `cluster::SetColour`, `cluster::ApplyPreset`): confirm when lifting read-only on a production-flagged cluster, audit, the [`PrefsWriter`] port to the settings; the guard also re-checks the flag right before each request |
//! | [`sidebar`] | E06-S10 | [`review_access`](sidebar::review_access) (the rules reviews the cluster sidebar hides sections by, failing open) and [`discover_custom_resources`](sidebar::discover_custom_resources) |
//! | [`integrations`] | E06-S10 | [`IntegrationRegistry`] (stub): registered integrations and the sidebar sections they append after the core ones |
//! | [`actions`] | E07-S08 | row actions: [`RowActionRegistry`] / [`RowActions`] (the actions a table's context menu and the palette offer per kind, read from the `CommandBus` once), the `resource::Delete` handler, and [`DeleteFlow`] (plan and run a delete of one object or a selection through the guard, with per-object results) |
//! | [`audit`] | E06-S02 | [`AuditLog`]: redacted, batched, fail-closed audit appends through `StatePort` |
//! | [`store`] | E07-S01 | [`ResourceStore`]: the per-session cache over reflector, metadata and Table feeds keyed by (gvk, scope), ref-counted feeds with grace teardown and a [`FeedBudget`](store::FeedBudget) hook, in-app sort / filter / name-namespace-label indices, and [`Subscription`] streams of coalesced [`StoreDelta`]s; [`ResourceStores`] keeps one per connected session |
//! | [`exec`] | E09-S08 | [`ExecService`]: a shell (`bash`, else `sh`, probed with a quick exec), an attach or a command in a pod container over the `ExecPort`; [`PodContainers`] and [`ContainerPlan`] pick the container (the default-container annotation, the last choice per pod, a picker for several); the exec-class policy lives in the [`MutationGuard`] (blocked read-only by default, audited, no confirmation); E09-S10 adds debug containers (`pod::Debug`, a low-risk guarded mutation: [`DebugRequest`], [`plan_debug`], [`ExecService::open_debug`], [`DebugRunner`]) |
//! | [`logs`] | E08-S01 | [`LogService`](logs::LogService): bounded, batched log sessions over the `LogPort` (`LogSession`, `LogBuffer` ring with seq index and a truncated marker, `LogDeltas`, `Connecting` / `Streaming` / `Ended` / `Failed`), cancelled on drop; `logs::aggregate` (E08-S04): the pods of a workload, Service or selector merged by server timestamp into one session |
//! | [`context`] | E08-S09 | [`ContextRegistry`]: `@`-mentions routed to their [`ContextProviderPort`](oxikube_ports::ContextProviderPort) within a byte budget; [`LogContextProvider`] (`@logs`), [`selection_context`] (the viewer's selection as a block) and the [`PendingContext`] queue "Send to agent" fills until the agent panel exists |
//! | [`tools`] | E08-S09 | [`ToolRegistry`]: read-only tools by name, listed, hidden without their capabilities and invoked after a schema check; `tools::k8s::get_logs` (`k8s.get_logs`: a pod's or a selector's newest lines, bounded and redacted) |
//! | [`columns`] | E07-S02 | [`ColumnProvider`]: table columns and cells per kind. [`CoreColumns`] (a table-driven catalogue of ~40 core kinds; Ready / Status / Restarts from the domain view-models; CPU and memory as pending metrics hooks) and [`TableColumns`] (a Table feed's server columns, `priority > 0` as `wide`); [`Cell`]s carry text, a typed sort key and a tone |
//! | [`session::namespaces`] | E06-S07 | [`NamespaceService`](session::namespaces::NamespaceService): namespace selection remembered per cluster, favourites, the namespace list with the RBAC fallback, `namespace::*` commands |

pub mod actions;
pub mod audit;
pub mod catalog;
pub mod columns;
pub mod command_bus;
pub mod context;
pub mod exec;
pub mod guard;
pub mod integrations;
pub mod logs;
pub mod search;
pub mod session;
pub mod sidebar;
pub mod sources;
pub mod store;
pub mod tools;

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
    CommandBus, CommandContext, CommandHandler, CommandIndex, CommandInfo, CommandOutput,
    CommandRegistry, CommandTarget, DispatchContext, DispatchError, DuplicateCommand,
    HandlerContext, InvokeError, MemoryRecents, Outcome, RECENTS_CAPACITY, RecentsStore,
    RegisterError, Selection, Unavailable, commands_for,
};
pub use context::{ContextRegistry, LogContextProvider, PendingContext, selection_context};
pub use exec::{
    ContainerChoices, ContainerPlan, DEFAULT_DEBUG_START_TIMEOUT, DebugDefaults, DebugOpened,
    DebugPlan, DebugReport, DebugRequest, DebugRunner, ExecService, PodContainers, ShellOptions,
    check_name, plan_debug, split_command,
};
pub use guard::{
    Confirmation, ConfirmationRequest, ConfirmationToken, Mutation, MutationGuard, PrefsPatch,
    PrefsWriter,
};
pub use integrations::{IntegrationRegistry, IntegrationSection, RegisterIntegrationError};
pub use search::aliases::{
    AliasConflict, AliasEntry, AliasFollow, AliasRegistry, AliasSource, AliasTable, ConflictKind,
    Resolution,
};
pub use search::fuzzy::{FuzzyService, Match, QueryGeneration};
pub use search::recents::{COMMAND_CAPACITY, JumpHistory, RecentList, StateRecents};
pub use session::{
    ClusterSession, ClusterSessionManager, SessionChange, SessionUpdate, SessionUpdates,
};
pub use sidebar::{AccessOutcome, CustomKind, CustomResourceGroup};
pub use sources::{KubeconfigSourcesService, SourceListStore, SourceRow};
pub use store::{
    CountState, CountTarget, CountsLease, KindCount, ResourceStore, ResourceStores, StoreDelta,
    StoreQuery, Subscription,
};
pub use tools::ToolRegistry;
