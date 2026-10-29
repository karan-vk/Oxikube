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
//! | [`access`] | E17-S01 | [`SubjectIndex`](access::SubjectIndex): the cluster's Roles, ClusterRoles and bindings from the store's subscriptions, kept incrementally as subject -> bindings -> normalised rules (`oxikube_domain::rbac`); `rules_for`, `bindings_for`, `subjects`, `can`, `roles_granting` and the k9s-style `matrix` |
//! | [`metrics_history`] | E13-S08 | [`MetricsHistoryService`]: the history graphs of a detail panel's Metrics tab: [`charts_for`] (the charts of each of the six kinds, as data), the range fetch through `PromqlPort` and the stack's query catalogue, reference lines, the remembered range |
//! | [`cloud`] | E18-S05 | [`CloudDiscoveryService`]: cloud clusters through the `CloudDiscoveryPort` adapters, refreshed off the UI thread with cancellation (one task per provider, scopes fanned out with a cap, per-scope failures kept apart), cached in the state db with a TTL (stale-while-revalidate), and the added ones merged into `<config dir>/kubeconfigs/<cli>.yaml` (atomic, `0600`, exec auth only), listed as kubeconfig sources so the catalog hot-reloads |
//! | [`catalog`] | E06-S03 | [`ClusterCatalog`]: the kubeconfig contexts with favourites and last-used, and [`ClusterCommands`], the handler of `cluster::Connect`, `cluster::Disconnect` and `cluster::ToggleFavourite` |
//! | [`search`] | E11-S04, E11-S11 | [`search::fuzzy`]: [`FuzzyService`], the one fuzzy ranking engine; [`search::recents`]: [`StateRecents`] and [`JumpRecents`], persisted through `StatePort`; [`search::aliases`]: [`AliasTable`], the words the `:` jump bar understands (built-in k9s aliases, aliases derived from API discovery, the user's `aliases.json`, collisions listed, never silent), and [`AliasRegistry`], one table per cluster kept in step with the sessions' discovery |
//! | [`session`] | E06-S01 | [`ClusterSessionManager`]: connect / disconnect / reconnect, the session state machine, capabilities, namespace selection, read-only flag, colour, and the [`SessionUpdates`] stream |
//! | [`session`] `prefs` | E06-S08 | per-cluster settings: `set_prefs_table` pushes the resolved `clusters.<id>` values; new sessions start from them, open ones follow them live |
//! | [`sources`] | E06-S05 | [`KubeconfigSourcesService`]: the user's kubeconfig sources (add a file or folder, paste, remove, reload) over the settings list, the `ClusterSourcePort` and `FsPort`, and the `kubeconfig::*` command handlers |
//! | [`command_bus`] | E06-S02 | [`CommandBus`]: dispatch by command id, the per-crate [`CommandRegistry`], MCP tool stubs |
//! | [`command_bus`] `availability`, `index` | E11-S01 | introspection for the palette, help overlay and keymap: [`CommandBus::list`] (the commands runnable in a [`CommandContext`]), `all`, `get`; [`CommandInfo`] / [`CommandIndex`] (sorted once, category then title), [`Selection`] and [`Unavailable`] |
//! | [`guard`] | E06-S02 | [`MutationGuard`]: read-only check, confirmation tier and token, dry-run stage (stub), the [`Mutation`] permit, audit |
//! | [`guard::posture`] | E06-S09 | read-only mode, colour and presets as commands (`cluster::ToggleReadOnly`, `cluster::SetColour`, `cluster::ApplyPreset`): confirm when lifting read-only on a production-flagged cluster, audit, the [`PrefsWriter`] port to the settings; the guard also re-checks the flag right before each request |
//! | [`sidebar`] | E06-S10 | [`review_access`](sidebar::review_access) (the rules reviews the cluster sidebar hides sections by, failing open) and [`discover_custom_resources`](sidebar::discover_custom_resources) |
//! | [`kinds`] | E12-S01 | [`KindRegistry`]: each kind's [`KindMeta`] (columns, actions as command ids, sidebar slot, templates) registered by its module, looked up in O(1) with a generic fallback; [`SidebarTree`] by section rank; [`KindGate`]: discovery presence ([`ServedKinds`]) and the [`KindVisibility`] hook E17 implements |
//! | [`integrations`] | E06-S10 | [`IntegrationRegistry`] (stub): registered integrations and the sidebar sections they append after the core ones |
//! | [`actions`] | E07-S08 | row actions: [`RowActionRegistry`] / [`RowActions`] (the actions a table's context menu and the palette offer per kind, read from the `CommandBus` once), the `resource::Delete` handler, and [`DeleteFlow`] (plan and run a delete of one object or a selection through the guard, with per-object results) |
//! | [`apply`] | E10-S08 | [`ApplyService`]: one object's manifest dry-run, previewed and applied through the guard (server-side apply under a field manager, conflicts with their managers and a force option, the replace fallback at the reviewed `resourceVersion`); the `resource::Apply` handler; Secret values masked |
//! | [`templates`] | E10-S09 | [`TemplateService`]: the templates of the "Create resource" picker (the embedded manifests of the core kinds passed in by the editor, plus `~/.config/oxikube/templates/*.yaml` read through `FsPort` each time the picker opens, a user template overriding an embedded one of the same name, a broken file skipped with a warning) and [`Template::prefill`] (the selection's namespace in the `namespace: default` placeholders) |
//! | [`ingress_class`] | E12-S14 | the Network section's one mutation: the `ingressclass::SetDefault` handler (annotation merge patch through the guard, the previous default cleared) |
//! | [`audit`] | E06-S02 | [`AuditLog`]: redacted, batched, fail-closed audit appends through `StatePort` |
//! | [`store`] | E07-S01 | [`ResourceStore`]: the per-session cache over reflector, metadata and Table feeds keyed by (gvk, scope), ref-counted feeds with grace teardown and a [`FeedBudget`](store::FeedBudget) hook, in-app sort / filter / name-namespace-label indices, and [`Subscription`] streams of coalesced [`StoreDelta`]s; [`ResourceStores`] keeps one per connected session |
//! | [`exec`] | E09-S08 | [`ExecService`]: a shell (`bash`, else `sh`, probed with a quick exec), an attach or a command in a pod container over the `ExecPort`; [`PodContainers`] and [`ContainerPlan`] pick the container (the default-container annotation, the last choice per pod, a picker for several); the exec-class policy lives in the [`MutationGuard`] (blocked read-only by default, audited, no confirmation); E09-S10 adds debug containers (`pod::Debug`, a low-risk guarded mutation: [`DebugRequest`], [`plan_debug`], [`ExecService::open_debug`], [`DebugRunner`]) |
//! | [`transfer`] | E20-S06 | [`TransferService`]: pod file download over the exec tar helpers (`oxikube_ports::ExecTransferExt`, with the `cat` fallback for one file in a container without `tar`) and the upload, which takes the guard's [`Mutation`]; [`safe_unpack`] (a tar stream onto disk through `FsPort`, refusing absolute paths, `..` and links) and [`tar_file_stream`] (one local file as a tar stream) |
//! | [`logs`] | E08-S01 | [`LogService`](logs::LogService): bounded, batched log sessions over the `LogPort` (`LogSession`, `LogBuffer` ring with seq index and a truncated marker, `LogDeltas`, `Connecting` / `Streaming` / `Ended` / `Failed`), cancelled on drop; `logs::aggregate` (E08-S04): the pods of a workload, Service or selector merged by server timestamp into one session |
//! | [`context`] | E08-S09 | [`ContextRegistry`]: `@`-mentions routed to their [`ContextProviderPort`](oxikube_ports::ContextProviderPort) within a byte budget; [`LogContextProvider`] (`@logs`), [`MetricsContextProvider`] (`@metrics/pod/<ns>/<name>`, E13-S12), [`selection_context`] (the viewer's selection as a block) and the [`PendingContext`] queue "Send to agent" fills until the agent panel exists |
//! | [`events`] | E14-S02 | [`EventService`]: every cluster's events from one feed per cluster in a bounded [`EventBuffer`] (`events.buffer`, default 10 000) de-duplicated by [`EventKey`] (count and last seen updated in place) and indexed by involved object, namespace and warnings; [`EventFilter`] queries newest first, [`EventsSubscription`] streams coalesced [`EventsChanged`]; it is the overview's [`RecentWarnings`] |
//! | [`warning_badges`] | E14-S04 | [`WarningBadges`]: the newest `Warning` per involved object within `events.warning_window` (default 1 h), kept from a warnings-only [`EventService`] subscription ([`BadgeTrack`]) and read `O(1)` per table row ([`ClusterBadges`]), [`BadgeChanges`] to redraw, [`ObjectEventsFeed`] for a resource's Events tab, and the Workloads overview's [`RecentWarnings`] over the window |
//! | [`notifications`] | E14-S05 | [`NotificationService`]: the in-app notification centre: each open cluster's Warning events of the watched kinds (from the [`EventService`], no watch of its own) through [`NotificationRules`] (kinds, reasons ignored), the per-cluster mute (persisted through `StatePort`), burst grouping ("12 pods: BackOff"), dedupe and a rate limit into a bounded read/unread list; `notifications::*` commands; the raised ones go to the OS notifier through `forward_to_os` |
//! | [`tools`] | E08-S09 | [`ToolRegistry`]: read-only tools by name, listed, hidden without their capabilities and invoked after a schema check; `tools::k8s::get_logs` (`k8s.get_logs`: a pod's or a selector's newest lines, bounded and redacted); `tools::metrics` (E13-S12: `k8s.top_pods`, `k8s.top_nodes`, `k8s.query_promql`, bounded, with absence as a visible `unavailable` result); `tools::events` (E14-S08: `k8s.get_events`, the `@events` mention and `events::SendToAgent`, read from the [`EventService`], bounded and redacted, the feed started on demand) |
//! | [`rollout`] | E12-S06 | [`RolloutService`]: a Deployment's, StatefulSet's or DaemonSet's revision history (ReplicaSets or ControllerRevisions, read on demand through the pure algorithm of `oxikube_domain::rollout`) and `kubectl rollout undo` as the guarded `workload::Rollback` (medium risk, a server dry run first, audit with `from=` and `to=`); [`RollbackRunner`] runs it from a dialog |
//! | [`workloads`] | E12-S08, E12-S04, E12-S03 | [`WorkloadService`]: trigger a CronJob (a Job from its template, `kubectl create job --from`), suspend and resume CronJobs and Jobs, and scale the replica controllers (`workload::Scale`, low risk, medium for zero: [`WorkloadService::plan_scale`] reads the `scale` subresource and any autoscaler, [`WorkloadService::scale`] patches `spec.replicas` through it, [`ScaleRunner`] runs it from a dialog); the `cronjob::*`, `job::*` and `workload::Scale` handlers (guarded, audited with the Job's name or the replica counts); evict, delete, force delete and sweep pods (`pod::Evict`, `pod::Delete`, `pod::ForceDelete`, `pod::Sanitize`: medium, high for the force delete, one audit record per pod under a batch id); the pod panel's reads (owner chain, node, environment sources) |
//! | [`helm`] | E16-S03 | [`HelmService`]: a release revision's manifest parsed once (off the UI thread, cached) into [`ResourcePlan`]s of `ResourceRef`s; [`ReleaseResourcesLease`] holds the store feeds while the Resources tab is open; [`ReleaseResource`] is `Present { health }`, `Missing` or `Unknown` |
//! | [`helm_charts`] | E16-S08 | [`ChartsService`]: the charts catalog over `HelmPort` (the whole catalog read once per generation and filtered in memory per keyword, grouped per chart with semver-ordered versions; only charts of configured repositories are shown; chart README / values / metadata cached per version; both dropped when a repository changes; readiness: no helm / no repos / ready; the hand-off to the install tab), `helm::SearchCharts` / `ShowChart` / `InstallChart` commands |
//! | [`sa_kubeconfig`] | E17-S09 | the guarded `serviceaccount::GenerateKubeconfig` (risk low, one confirmation): a `TokenRequest` (legacy token Secret on clusters without it, only when allowed) turned into a kubeconfig held in memory by the `KubeconfigVault`; audited without the token |
//! | [`helm_ops`] | E16-S06 | the guarded `helm::Rollback` (risk medium, one confirmation) and `helm::Uninstall` (risk high, type the release name; keep-history option); `values_diff` for the rollback preview; audited without values |
//! | [`role_ops`] | E17-S05 | the guarded `role::Create`, `role::Edit` and `role::Delete` (risk medium, one confirmation, a server dry run first, writing Roles and ClusterRoles only; audited with a rule count) and [`RoleLinks`](role_ops::RoleLinks): the sources of an aggregated ClusterRole and the bindings that name a role |
//! | [`binding_ops`] | E17-S06 | the guarded `binding::Create`, `binding::Edit` and `binding::Delete` (risk medium, one confirmation that calls out `cluster-admin` and everybody-groups, a server dry run first, writing RoleBindings and ClusterRoleBindings only; a changed `roleRef` is a delete and a create as one intent; audited with the role and a subject count) and [`BindingChoices`](binding_ops::BindingChoices): what the dialog's pickers offer |
//! | [`helm_install`] | E16-S09 | [`InstallFlow`]: the install tab's guarded `helm::Install` (preview = the guard's dry run: `helm template` plus the server's dry run, Secret data masked; ask = the low-risk confirmation; start = the install with its output followed line by line through [`ReleaseProgress`]; Cancel kills `helm`); audited without the values |
//! | [`helm_upgrade`] | E16-S10 | [`UpgradeFlow`]: the upgrade tab's guarded `helm::Upgrade` (a release's current chart and values read natively; preview = the guard's dry run: `helm template` with the values, the server's dry run, the objects added / removed / changed against the current manifest; ask = the medium-risk confirmation; start = the upgrade with its output followed line by line; Cancel kills `helm`); `helm::UpgradeRelease` hands a release to the tab; audited with the chart versions and `reuse_values`, never the values |
//! | [`helm_releases`] | E16-S05 | [`ReleasesService`]: a cluster's Helm releases for the Releases view, read natively (list, history, values, manifest, notes, the E16-S03 resources join), credential-looking values and Secret manifests masked until the user's `helm::RevealValues`, a live watch on the release Secrets and ConfigMaps, the `/` filter over rows; `helm::ShowValues` / `ShowManifest` / `RevealValues` commands |
//! | [`rbac_subjects`] | E17-S07 | [`SubjectsService`]: the subject views of RBAC. [`SubjectIndex`](access::SubjectIndex) groups the cluster's bindings by user, ServiceAccount and group (counts of bindings and roles; the API cannot list a group's members), and projects one subject's policy as k9s's per-verb matrix by API group ([`SubjectMatrix`]) cluster-wide or per namespace, each cell traced to the bindings and roles that grant it ([`Provenance`](rbac_subjects::Provenance)), with tab-separated export and the `/` filter; the four lists are read concurrently off the UI thread and the matrix is computed per selected subject |
//! | [`helm_repos`] | E16-S07 | [`HelmReposService`]: the chart repositories in Helm's own configuration (list, add, remove, update one at a time through one queue, per-repository refresh status, a cue for the charts catalog to drop cached searches, memory-only staged passwords, readable errors, quick-add suggestions), `helm::RepoAdd` / `RepoRemove` / `RepoUpdate` commands; an agent's add needs permission |
//! | [`portforward`] | E15-S03 | [`PortForwardManager`]: port-forwards to pods and Services across clusters (a registry keyed by [`ForwardId`], Service to ready pod through EndpointSlices preferring the previous pod, the target watched through the store, a pure restart state machine with backoff, the same local port across reconnects), drops / recoveries / failures to the notification centre, the read-only policy, a coalesced change subscription, `portforward::*` commands and the `service::OpenInBrowser` opener; E15-S06 `portforward::favourites`: [`PortForwardFavourites`](portforward::PortForwardFavourites), saved forwards in the state db's `port_forward_favourites` table, restored (4 at a time, failures to the notification centre) when a session becomes `Ready` |
//! | [`overview`] | E12-S07 | the Workloads overview's inputs outside the store: [`RecentWarnings`], the hook the Events feed (E14) implements for the warnings panel, with [`NoRecentWarnings`] until then; the counters themselves are [`WorkloadCounters`] and the status buckets of `store::counts` |
//! | [`selection`] | E12-S19 | The bulk actions on a selection: `selection::Delete`, `selection::Restart` and `selection::Label` (labels or annotations); one guarded command each, one confirmation at the riskiest object's tier, a result and an audit record per object under a batch id; [`SelectionFlow`] plans and runs them for a dialog |
//! | [`cluster_overview`] | E13-S09 | the cluster overview's numbers: [`OverviewFeed`](cluster_overview::OverviewFeed) (pods by phase, nodes by readiness, capacity and allocation, live usage, top consumers, kept incrementally off the UI thread and published as an [`OverviewSnapshot`](cluster_overview::OverviewSnapshot)), [`detect_distribution`](cluster_overview::detect_distribution) and the Prometheus graphs' [`fetch_cluster_history`](cluster_overview::fetch_cluster_history) |
//! | [`network`] | E12-S13 | [`ServiceBackends`]: the label-selected store queries of a Service's detail (its EndpointSlices and its pods); [`plan_show_pods`], `service::ShowPods` as a jump; [`ServicePortOpener`], the hook the port-forward manager implements for `service::OpenInBrowser`, registered only when there is one |
//! | [`secrets`] | E12-S11 | [`SecretService`]: decode a Secret's values in memory, encode them back, summarise a Secret without its values ([`SecretSummary`]: key names and sizes, registry logins without passwords, a TLS certificate's subject and expiry), validate and build a new Secret ([`NewSecret`]: Opaque, Docker registry, TLS), and the manifest of a decoded editor re-encoded to base64 for `resource::Apply`; `tools::k8s::get_secret` (`k8s.get_secret`: masked unless a [`RevealPolicy`](tools::k8s::RevealPolicy) allows values, denied by default) |
//! | [`columns`] | E07-S02 | [`ColumnProvider`]: table columns and cells per kind. [`CoreColumns`] (a table-driven catalogue of ~40 core kinds; Ready / Status / Restarts from the domain view-models; CPU and memory as pending metrics hooks) and [`TableColumns`] (a Table feed's server columns, `priority > 0` as `wide`); [`Cell`]s carry text, a typed sort key and a tone |
//! | [`allocation`] | E13-S11 | [`AllocationView`](allocation::AllocationView): the rows of a node's or namespace's allocation section (requests, limits and live usage against allocatable or a `ResourceQuota`, GPUs, the pods that set none), with the text made, over the S02 sums; live usage from the metrics port or `metrics unavailable: <reason>` |
//! | [`metrics`] | E13-S02 | [`UtilisationAggregator`]: per-pod `%CPU/R`-style percents, per-node ([`NodeAllocation`]) and per-namespace ([`NamespaceAllocation`]) requests and limits over resource snapshots, with the hot-reloadable GPU vendor list ([`GpuVendorsHandle`], `metrics.gpu_vendors`); the pure maths is `oxikube_domain::utilisation` |
//! | [`metrics::live`] | E13-S03 | [`MetricsService`]: per-cluster metrics-server polling (default 15 s, backoff on error, paused with no visible consumer), [`ClusterUsage`] (latest sample and a 60-sample ring per node and pod; the Pods and Nodes tables' metrics source), [`MetricsState`] (`metrics unavailable: <reason>`) |
//! | [`namespaces`] | E12-S16 | the Namespace kind: `namespace::Create` / `namespace::Delete` handlers (guarded), [`delete_advice`](namespaces::delete_advice) (HNC and system-namespace warnings, counts of what a namespace holds) and [`NamespaceColumns`](namespaces::NamespaceColumns) (a live Pods count from the pods feed's per-namespace index) |
//! | [`session::namespaces`] | E06-S07 | [`NamespaceService`](session::namespaces::NamespaceService): namespace selection remembered per cluster, favourites, the namespace list with the RBAC fallback, `namespace::*` commands |

pub mod access;
pub mod actions;
pub mod allocation;
pub mod apply;
pub mod audit;
pub mod binding_ops;
pub mod catalog;
pub mod cloud;
pub mod cluster_overview;
pub mod columns;
pub mod command_bus;
pub mod context;
pub mod events;
pub mod exec;
pub mod guard;
pub mod helm;
pub mod helm_charts;
pub mod helm_install;
pub mod helm_ops;
pub mod helm_releases;
pub mod helm_repos;
pub mod helm_upgrade;
pub mod ingress_class;
pub mod integrations;
pub mod kinds;
pub mod kustomize;
pub mod logs;
pub mod metrics;
pub mod metrics_history;
pub mod metrics_provider;
pub mod namespaces;
pub mod network;
pub mod nodes;
pub mod notifications;
pub mod os_notifications;
pub mod overview;
pub mod portforward;
pub mod rbac_subjects;
pub mod references;
pub mod role_ops;
pub mod rollout;
pub mod sa_kubeconfig;
pub mod search;
pub mod secrets;
pub mod selection;
pub mod session;
pub mod sidebar;
pub mod sources;
pub mod storage;
pub mod store;
pub mod templates;
pub mod tools;
pub mod transfer;
pub mod warning_badges;
pub mod workloads;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod testing_posture;

pub use actions::{
    ActionAccess, ActionContext, ActionState, DeleteError, DeleteFlow, DeletePlan, DeleteReport,
    ItemResult, ItemStatus, ResolvedAction, RowAction, RowActionRegistry, RowActionSpec,
    RowActions, object_label,
};
pub use apply::{
    ApplyError, ApplyMethod, ApplyPreview, ApplyReport, ApplyRequest, ApplyService, BatchCounts,
    BatchPreview, BatchReport, Change, DocumentPreview, DocumentResult, DocumentStatus,
    FieldConflict, PreviewOutcome,
};
pub use audit::AuditLog;
pub use catalog::{
    CatalogEntry, ClusterCatalog, ClusterCommandOutcome, ClusterCommands, FavouriteChanged,
    FavouritesLagged,
};
pub use cloud::{CloudDiscoveryEvent, CloudDiscoveryService};
pub use columns::{Cell, Column, ColumnId, ColumnProvider, CoreColumns, PodColumns, TableColumns};
pub use command_bus::{
    CommandBus, CommandContext, CommandHandler, CommandIndex, CommandInfo, CommandOutput,
    CommandRegistry, CommandTarget, DispatchContext, DispatchError, DuplicateCommand,
    HandlerContext, InvokeError, MemoryRecents, Outcome, RECENTS_CAPACITY, RecentsStore,
    RegisterError, Selection, Unavailable, commands_for,
};
pub use context::{
    ContextRegistry, LogContextProvider, MetricsContextProvider, PendingContext, selection_context,
};
pub use events::{
    EventBuffer, EventFilter, EventFilterError, EventKey, EventService, EventsChanged,
    EventsConfig, EventsState, EventsSubscription, ObjectIdentity, ObjectTarget,
};
pub use exec::{
    ContainerChoices, ContainerPlan, DEFAULT_DEBUG_START_TIMEOUT, DebugDefaults, DebugOpened,
    DebugPlan, DebugReport, DebugRequest, DebugRunner, ExecService, PodContainers, ShellOptions,
    check_name, plan_debug, split_command,
};
pub use guard::{
    Confirmation, ConfirmationRequest, ConfirmationToken, Mutation, MutationGuard, PrefsPatch,
    PrefsWriter,
};
pub use helm::{
    HelmService, KindScopes, PlannedResource, ReleaseResource, ReleaseResourcesLease, ResourcePlan,
    ResourceState,
};
pub use helm_charts::{ChartsService, Readiness as ChartsReadiness};
pub use helm_install::{
    InstallError, InstallFlow, InstallPreview, InstallSpec, InstallStart, ReleaseProgress,
};
pub use helm_releases::{ReleaseSource, ReleasesService};
pub use helm_repos::{
    HelmReposService, RepoActivity, RepoRow, RepoSnapshot, RepoStatus, UpdateReport,
};
pub use helm_upgrade::{
    CurrentRelease, UpgradeError, UpgradeFlow, UpgradePreview, UpgradeRequests, UpgradeSpec,
    UpgradeStart, UpgradeTarget,
};
pub use integrations::{IntegrationRegistry, IntegrationSection, RegisterIntegrationError};
pub use kinds::{
    ColumnsContext, ColumnsFactory, GroupKind, KindColumns, KindGate, KindMeta, KindRegistry,
    KindVisibility, RegisterKindError, Section, ServedKinds, SidebarSlot, SidebarTree,
};
pub use metrics::{
    ClusterUsage, ClusterUtilisation, GpuVendorsHandle, LiveMetricsConfig, LiveMetricsConfigTable,
    MetricsInterest, MetricsKind, MetricsService, MetricsState, MetricsVisibility,
    NamespaceAllocation, NodeAllocation, ResourceUse, UtilisationAggregator, allocate_namespaces,
    allocate_nodes, cluster_utilisation,
};
pub use metrics_history::{
    ChartData, ChartDef, HistoryAvailability, HistoryError, HistorySubject, MetricsHistoryService,
    SubjectKind, charts_for,
};
pub use metrics_provider::{EndpointSink, MetricsProviderService, ProviderState, ProviderStatus};
pub use network::{
    OpenedServicePort, PodsPlan, ServiceBackends, ServicePortOpener, UrlSink, plan_show_pods,
    register_open_in_browser,
};
pub use notifications::{NotificationRules, NotificationService};
pub use os_notifications::{ClickAction, Delivery, OsNotificationService};
pub use overview::{FixedWarnings, NoRecentWarnings, RecentWarnings, WarningRow};
pub use portforward::annotations::{
    AnnotationSuggestion, SuggestedForward, annotation_suggestions, parse_k9s_forwards,
};
pub use portforward::{
    ForwardBackend, ForwardChanges, ForwardConfig, ForwardId, ForwardInfo, ForwardStarter,
    ForwardState, PortForwardManager, SessionBackend, StartError, StartedForward,
};
pub use rbac_subjects::{SubjectMatrix, SubjectsService};
pub use references::{
    OwnerCache, PodKey, PodUse, ReferenceFeed, ReferenceHub, ReferenceIndex, Usage, UsageStatus,
    UsedBy, WorkloadUse, used_by,
};
pub use rollout::{
    RollbackOutcome, RollbackReport, RollbackResult, RollbackRunner, RolloutHistory, RolloutService,
};
pub use search::aliases::{
    AliasConflict, AliasEntry, AliasFollow, AliasRegistry, AliasSource, AliasTable, ConflictKind,
    Resolution,
};
pub use search::fuzzy::{FuzzyService, Match, QueryGeneration};
pub use search::recents::{JumpRecents, RecentList, StateRecents};
pub use secrets::{
    Field as NewSecretField, KeyInfo, NewSecret, PemProblem, Problem as NewSecretProblem,
    SecretService, SecretSummary, TlsInfo, TypeSummary,
};
pub use selection::{
    BulkReport, BulkRow, BulkStatus, SelectionError, SelectionFlow, SelectionPlan,
};
pub use session::{
    ClusterSession, ClusterSessionManager, SessionChange, SessionUpdate, SessionUpdates,
};
pub use sidebar::{AccessOutcome, CustomKind, CustomResourceGroup};
pub use sources::{KubeconfigSourcesService, SourceListStore, SourceRow};
pub use storage::{
    DefaultClassContext, DefaultClassFlow, DefaultClassReport, PODS_COLUMN, PvcColumns,
    ResizeContext, ResizeFlow, ResizeReport,
};
pub use store::{
    BucketCounts, CORE_TARGETS, CoreTarget, CountState, CountTarget, CountsLease, FetchedCount,
    KindCount, ResourceStore, ResourceStores, StoreDelta, StoreQuery, Subscription,
    WorkloadCounters, fetch_count,
};
pub use templates::{
    Prefill, SkippedTemplate, Template, TemplateCatalog, TemplateOrigin, TemplateService,
};
pub use tools::ToolRegistry;
pub use transfer::{
    Download, DownloadFormat, RemoteEntry, Skipped, TransferService, UnpackReport, UploadReceipt,
    safe_unpack, tar_file_stream,
};
pub use warning_badges::{
    BadgeChanges, BadgeTrack, ClusterBadges, DEFAULT_WARNING_WINDOW, ObjectEventsFeed,
    WarningBadge, WarningBadges,
};
pub use workloads::{
    MAX_REPLICAS, RestartResult, ScalePlan, ScaleReport, ScaleResult, ScaleRunner, SetImagePlan,
    SetImageReport, SetImageResult, SetImageRunner, SuspendOutcome, Triggered, WorkloadService,
    is_rollable, is_scalable, parse_replicas, replicas_word,
};
