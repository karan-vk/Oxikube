# Domain glossary (CONTEXT.md)

Shared vocabulary. Type names in code match these terms; if you need a new term, add it here first.

How to read the **Lives in** column:

- A path under `oxikube_domain` or `oxikube_ports` names a module that exists today. The
  `tests/glossary.rs` test in `oxikube_testkit` fails when such a module is renamed or removed.
- `oxikube_testkit` paths exist today.
- Paths in `oxikube_app`, adapters, `platform` and `ui` crates, and anything marked *(planned)*, are
  the agreed home of a term whose owning epic has not landed yet. Do not invent a different home;
  change this table in the same PR that moves it.

## Identity and resources

| Term | Meaning | Lives in |
|---|---|---|
| **ClusterId** | Stable identifier for a cluster entry in the catalog (hash of kubeconfig source + context name). | `oxikube_domain::ids` |
| **ContextName** | A kubeconfig context name. | `oxikube_domain::ids` |
| **Gvk / Gvr** | Group-Version-Kind / Group-Version-Resource. | `oxikube_domain::ids` |
| **Scope** | Whether a kind is `Namespaced` or `Cluster` scoped. | `oxikube_domain::ids` |
| **ResourceRef** | `{cluster, gvk, namespace, name}` — the address of one object. | `oxikube_domain::ids` |
| **ResourceKind** | Discovery record for one served kind: `Gvk`, plural/singular names, short names, categories, `VerbSet` of supported `Verb`s, namespaced flag, preferred version. Filled by API discovery, read by tables, palette and sidebar. | `oxikube_domain::kinds` |
| **Resource** | Thin model: `ObjectMeta` (with `OwnerRef`s) + raw `serde_json::Value`. The domain never depends on k8s-openapi. A metadata-only object (from a metadata feed) is **partial** (`Resource::is_partial`): no `spec`/`status`/data, never to be rendered as complete; `KubeResources::upgrade` fetches the full one. | `oxikube_domain::resource` |
| **View-model** | Typed projection built from `Resource` JSON for a core kind (`PodSummary` with `ContainerSummary`, `NodeSummary`, `WorkloadSummary`, `JobSummary`, `CronJobSummary`). | `oxikube_domain::view` |
| **Quantity** | A Kubernetes resource quantity (`500m`, `128Mi`) parsed into an exact decimal (`i128` mantissa × 10^exponent, nano precision), printed like apimachinery `Quantity.String()`. | `oxikube_domain::quantity` |
| **Age** | A non-negative span since a resource was created, formatted `kubectl`-style (`AgeStyle::Kubectl`) or kdash-style (`AgeStyle::Detailed`). | `oxikube_domain::age` |
| **LogLine** | One line of container output with its kubelet timestamp; over-long lines are cut and flagged. | `oxikube_domain::log` |
| **Event** | A Kubernetes Event (`EventType` Normal/Warning, reason, message, the `regarding` object). | `oxikube_domain::event` |
| **MetricsSample** | One CPU and memory reading for a node, pod or container (`MetricsSubject`); an absent value is `Reading::Missing` with a `MissingReason`. | `oxikube_domain::metrics` |

## Errors

| Term | Meaning | Lives in |
|---|---|---|
| **OxiError / OxiResult** | The error every port returns: `{kind, message, source, retryable}`. Branch on `kind()` and `is_retryable()`, never on the message. | `oxikube_domain::error` |
| **ErrorKind** | `Auth`, `Forbidden`, `NotFound`, `Conflict`, `Network`, `Timeout`, `Validation`, `Unsupported`, `Internal`, `BudgetExceeded` (the watch budget refused a new feed). Adapter mapping rules are in `docs/ARCHITECTURE.md`. | `oxikube_domain::error` |
| **Redaction** | Removing tokens and Secret data from text before it is logged, audited or put in an error. `oxikube_domain::redact` holds the pure scrubber (`redact(&str) -> Cow<str>`, `Redacted<T>`); adapters redact before building an error and `oxikube_logging` scrubs every log line with it. | `oxikube_domain::redact` |

## Sessions and state

| Term | Meaning | Lives in |
|---|---|---|
| **ClusterSession** | One open cluster: its `ClusterId` and context, `ClusterSessionState`, the `ClusterPorts` bundle while connected, `Capabilities`, `NamespaceSelection`, read-only flag and `ClusterColour`. One workspace tab per session. Owned by the **ClusterSessionManager** (connect / disconnect / reconnect, `SessionUpdate` stream); `ClusterSession` is a snapshot of it that hands out the read half of the resource port only. | `oxikube_app::session` |
| **ClusterConnectorPort / ClusterPorts** | The port that turns a context into a live connection, and the per-connection bundle it returns (resources, discovery, tables, logs, exec, port-forward, metrics, `AccessReviewPort`). The adapter reports health through a `HealthReporter` (`HealthSignal`: `Healthy`, `Unhealthy`, `Failed`); dropping the `ClusterConnection` tears its feeds and probe loop down. `ExecInteractivity` is the per-cluster exec-plugin policy passed with each connect. | `oxikube_ports::{connector, access}` |
| **SessionUpdate** | One change to one session (`Opened`, `StateChanged`, `CapabilitiesChanged`, `NamespaceChanged`, `ReadOnlyChanged`, `ColourChanged`, `Closed`), tagged with its `ClusterId`, on a bounded broadcast; a slow subscriber gets `SessionLagged` and re-reads. Not to be confused with `SessionEvent`, the state machine's input. | `oxikube_app::session` |
| **Catalog / CatalogEntry** | The list of every kubeconfig context Oxikube knows (the home screen). A `CatalogEntry` is one `ClusterContext` of the `ClusterSourcePort` (name, kubeconfig cluster and user names, source file, an optional `problem` when it names something the kubeconfig lacks) plus the user's marks: favourite and last-used time, kept in the `cluster_catalog` state table. Ordered favourites, then last used, then name. Reading it never touches a cluster. | `oxikube_app::catalog` |
| **ClusterColour** | A cluster's accent colour, sRGB written `#rrggbb` (hotbar dot, tab stripe, badges). | `oxikube_domain::colour` |
| **ClusterSessionState** | `Disconnected → Connecting → (AuthRequired) → Ready ↔ Degraded → Error`. Advanced by `SessionEvent`s; `SessionPhase` is the payload-free discriminant; a bad pair is an `InvalidTransition`. | `oxikube_domain::session` |
| **NamespaceSelection** | `All` or a `Set` of namespaces chosen in the UI. `NamespaceFavourites` is the user's pinned list. | `oxikube_domain::session` |
| **WatchScope** | Derived from a `NamespaceSelection` and a kind's `Scope`: `Cluster` or `Namespaces`; decides whether feeds are cluster- or namespace-scoped. | `oxikube_domain::session` |
| **NamespaceService** | Sets a session's `NamespaceSelection` and remembers it per cluster with the favourites and the namespace names the user typed (`NamespacePrefs`, `StatePort` kv `cluster/<id>/namespaces`, never settings). Lists namespaces for the selector; when the cluster answers `Forbidden` the list is the typed names (`NamespaceSource`). Runs the `namespace::Select` and `namespace::ToggleFavourite` commands. An empty selection is `All`; `0`-`9` select All or the first nine favourites. | `oxikube_app::session::namespaces` |
| **ScopeDelta** | What changes between two `WatchScope`s of a kind: namespaces whose feed starts, stops or stays, and whether the cluster-wide feed starts or stops. `{a, b}` to `{b, c}` starts `c`, stops `a`, keeps `b`. The `ResourceStore` re-scopes from it. | `oxikube_app::session::namespaces` |
| **NamespaceSelector** | The namespace dropdown of one cluster tab: trigger, search box, virtualised list of All / favourites / namespaces, digit keys. A view over the `NamespaceService`; it never opens feeds. | `oxikube_catalog_ui::namespaces` |
| **Feed** | A live stream of `DeltaBatch`es for one (cluster, gvk, scope): reflector, metadata-only, or Table API. Typed as `WatchFeed`. | `oxikube_ports::feed`; producers in `oxikube_kube::feed` (reflector and metadata-only variant: `ReflectorFeed`, E04-S02/S03) |
| **Watch budget / FeedStats** | Per-cluster limits on feeds (`max_feeds`) and held objects (`max_objects`), the idle grace period after which an unobserved feed is torn down, and the degrade of full feeds to metadata-only above `metadata_above` objects. `FeedRegistry` hands out `FeedLease`s (one per subscriber) and `SelectionLease`s (one feed per selected namespace); `FeedStats` is its counter snapshot (feeds, objects, events, restarts, bytes), `FeedVariant` what a feed carries (`Full`, `Metadata`, `Table`). | `oxikube_ports::feed_stats`; registry `oxikube_kube::budget` (E04-S13) |
| **FeedState** | Health of one feed: `Warming` (listing or relisting), `Live`, `Retrying` (backing off after a watch error), `Stopped`. Lets the session go `Degraded` while a feed retries. | `oxikube_kube::feed` |
| **Delta / DeltaBatch** | One change in a feed (`Applied`, `Deleted`, `Restarted`) and a coalesced batch of them. Lives in ports, not the domain: it is transport. | `oxikube_ports::feed` |
| **Table / TableFeed** | Server-side Table API data (kubectl-identical columns incl. CRD printer columns) as `TableColumn`s and `TableRow`s. | `oxikube_ports::table`; producer `oxikube_kube::table` |
| **Drain** | Emptying a node of its pods like `kubectl drain`: plan (which pods are evicted, skipped as DaemonSet or mirror pods, or block the drain), cordon, evict with retry while a PodDisruptionBudget refuses, wait for each pod to go. Runs as a stream of `DrainProgress` and ends in a `DrainSummary`; configured by `DrainOptions`. | `oxikube_kube::algorithms` |
| **RolloutRevision** | One entry of a Deployment's rollout history: the revision number, the ReplicaSet that holds its template, the change cause and the images. `rollout undo` restores the template of one. | `oxikube_kube::algorithms` |
| **TableSource** | Whether a `Table`'s columns are the server's (`Server`) or the adapter's plain-JSON fallback (`Objects`, the server ignored the Table `Accept` header). | `oxikube_ports::table` |
| **ResourceStore** | App-side cache over feeds with sort/filter/index and ref-counted subscriptions. | `oxikube_app::store` *(planned)* |
| **ColumnProvider** | Produces table columns + cells for a kind (core columns, Table API columns, metrics columns). | `oxikube_app::columns` *(planned)* |
| **KindSpec** | Per-kind registration: columns, detail renderer, actions, templates, sidebar section. | `oxikube_resources_ui::kinds` *(planned)* |

## Commands and safety

| Term | Meaning | Lives in |
|---|---|---|
| **Command** | A user/agent action: the `Command` payload enum, identified by a `CommandId` (`workload::Scale`) and described by `CommandMeta` (title, `CommandScope`, mutating, confirm tier, capabilities). Dispatched through the **CommandBus** (`CommandBus::dispatch(cmd, DispatchContext)`; handlers registered per crate into a `CommandRegistry`, each with its MCP tool stub). The id is also the keymap action and the MCP tool name. | `oxikube_domain::command`; bus in `oxikube_app::command_bus` |
| **Capability** | A flag for what a session/backend can do (mutate, exec, logs, portforward, helm, argo…). `Capabilities` is the set. | `oxikube_domain::command` |
| **MutationGuard** | The single pipeline every mutation passes: read-only check → confirmation tier → dry-run → execute → audit. It alone holds a `ResourceWriter` and hands it to a handler only inside a **Mutation** permit. A confirmation is a round trip: `Outcome::NeedsConfirmation` with a single-use `ConfirmationToken`, then a second dispatch carrying the `Confirmation`. Failing to audit fails the mutation closed. The writer inside a **Mutation** re-reads the read-only flag before every request, so a flow that outlives admission still stops. | `oxikube_app::guard`, `oxikube_app::audit` |
| **Risk / ConfirmTier** | Stories say "confirm low / medium / high". Mapping: **Low** → `Simple` (one-click confirm; may be skipped per session by setting), **Medium** → `Simple` naming the target and cluster, **High** → `TypeName`, **Irreversible** → `TypeName` + mandatory server dry-run diff. Reads never confirm. A read-only cluster refuses every tier before any request and records `AuditOutcome::Denied`; the guard returns the typed `DispatchError::ReadOnly { cluster, context }`, which maps to `ErrorKind::Forbidden` (`ErrorKind` has no `ReadOnly` variant). | `oxikube_domain::safety` |
| **Initiator** | `Ui / Command / Agent / Plugin` — who asked for a mutation. Defined in `safety`, re-exported from `audit`. | `oxikube_domain::safety` |
| **AuditRecord** | Who/what/where/when/outcome/initiator for a mutation (`AuditOutcome`: Succeeded, Failed, Denied, Cancelled). Never holds request bodies. Stored in SQLite through `StatePort`. | `oxikube_domain::audit` |

## Ports

A **port** is an async, object-safe trait in `oxikube_ports` that the app depends on and an adapter implements. Every fallible method returns `OxiResult`. Each port's module docs name its adapter; each has a `Fake*` in `oxikube_testkit::fakes`.

| Term | Meaning | Lives in |
|---|---|---|
| **ResourcePort** | `ResourceReader` + `ResourceWriter`. Only `MutationGuard` holds the writer half. | `oxikube_ports::resource` |
| **Forward** | One port-forward: a `ForwardSpec` (a pod or service `ResourceRef`, a remote `ForwardPort`, a local port and bind address, loopback by default) and its `ForwardStatus` (`Starting`, `Listening`, `TargetGone`, `Error`, `Stopped`). Run by the kube adapter, owned by `PortForwardManager` (E15). | `oxikube_domain::portforward`; adapter in `oxikube_kube::remote::portforward` |
| **DiscoveryPort / LogPort / ExecPort / PortForwardPort / TableFeedPort** | Data-plane access to a connected cluster. | `oxikube_ports::{discovery, log, exec, portforward, table}` |
| **Node shell / debug container** | Two ways into a workload that has no shell of its own, both run by `oxikube_kube::remote::exec` on top of `ExecPort`. A *node shell* is a privileged pod pinned to a node, exec'd into the node's namespaces and deleted when the shell ends, fails or is dropped (a labelled sweep removes leftovers). A *debug container* is an ephemeral container added to a running pod, then attached to. Both create objects, so they are guarded mutations. | `oxikube_kube::remote::exec::{node_shell, KubeExec::debug_container}` |
| **ClusterSourcePort / CloudDiscoveryPort** | Where cluster contexts come from: kubeconfig sources and cloud CLIs. | `oxikube_ports::{cluster_source, cloud}` |
| **MetricsPort / PromqlPort / DescribePort / HelmPort** | Optional read access: metrics-server, Prometheus, `describe`, Helm releases. | `oxikube_ports::{metrics, promql, describe, helm}` |
| **StatePort / SecretStorePort** | Durable local state (SQLite) and the only place secrets may be persisted (OS keychain). | `oxikube_ports::{state, secrets}` |
| **NotifierPort / UpdaterPort / CrashReporterPort / FsPort / ClockPort** | Platform services behind ports so tests can fake them. | `oxikube_ports::{notifier, updater, crash, fs, clock}` |
| **TerminalBackend** | Byte-stream source for the terminal element: local PTY, kube exec/attach, display-only. The trait is planned in E09-S01 beside `ExecPort`/`ExecSession` so adapters can implement it; the element that paints it lives in `oxikube_terminal`. | *(planned)* trait `oxikube_ports::exec`; impls in `oxikube_terminal` (local PTY, display-only), `oxikube_kube` (exec/attach), `oxikube_argocd` (terminal WS) |

## Integrations and agents

| Term | Meaning | Lives in |
|---|---|---|
| **Integration** | Optional capability pack (Argo CD, Flux) detected per cluster via `IntegrationPort`; contributes a declarative `SidebarModel`, commands, tools, context providers, settings. | `oxikube_ports::integration`, `oxikube_app::integrations` *(planned)* |
| **ArgoBackend** | CRD-direct / Server REST / Core helper implementations behind one trait with capability flags. | `oxikube_argocd` *(planned)* |
| **Tool / ToolDef** | An MCP-shaped capability (`ToolName`, JSON schema, `ToolAnnotations`, risk) behind `ToolPort`, held in the **ToolRegistry**; mutating tools are permission-gated. | `oxikube_ports::tool`, `oxikube_app::tools` *(planned)* |
| **ContextProvider / Mention** | `ContextProviderPort` resolves a `Mention` (`@kind/ns/name`, `@logs/...`, `@events/...`, claimed by a `MentionPrefix`) into **ContextBlock**s with size budgets. `ContentPart` is the text/image/resource shape shared by prompts and tool results. | `oxikube_ports::context`, `oxikube_app::context` *(planned)* |
| **ContextBlock** | A bounded piece of cluster context handed to an agent: title, MIME type, text body (capped at 64 KiB, `truncated` flag). Earlier docs called it ContentBlock. | `oxikube_domain::agent` |
| **AgentPort / AgentClient** | The ACP connection to a hosted agent (what Oxikube calls) and the callbacks the agent may make of Oxikube (permissions, fs, terminal, elicitation). | `oxikube_ports::agent` |
| **AgentSession / AgentThread** | An ACP session (`AgentSessionId`) with a hosted agent and its thread of entries (messages, thoughts, tool calls, plan, permissions), built from `SessionUpdate`s. The thread model is not written yet. | ids and updates in `oxikube_ports::agent`; thread *(planned)* `oxikube_domain::agent::thread`, `oxikube_app::agent` |
| **Extension** | A WASM component (WIT world `oxikube:extension`) contributing themes, commands, MCP servers; never UI. | `oxikube_extension_api`, `oxikube_extension_host` |

## Workspace shell, settings and testing

| Term | Meaning | Lives in |
|---|---|---|
| **Item / Panel / Pane / Dock** | Workspace shell concepts (Zed design): an **Item** is the content of a centre tab (title, icon, dirty; may refuse to close); a **Panel** is a dockable side view; a **Pane** is a tab strip of items, and the **PaneGroup** the tree of horizontal/vertical splits of panes in the centre; a **Dock** is the left, bottom or right edge area holding panels. Closed items are kept as descriptors (kind + state), not live entities, so they can be reopened. | `oxikube_workspace` |
| **Layout persistence** | Saving the workspace's layout (dock sizes and visibility, the split and tab tree, the open item descriptors `{ kind, state }`, the active pane, the window place) as a versioned `SerializedWorkspace` row in the `StatePort` table `workspace_layout`, one per window id, and restoring it on launch through the `ItemRegistry`. Unknown item kinds are skipped. A corrupt state database is moved aside and replaced; settings and keymap files are never touched. Never holds secrets. | `oxikube_workspace::persistence`, `oxikube_state_sqlite` |
| **ClusterTab / ClusterTabs** | The workspace tab of one cluster session: an `Item` hosting a `Workspace` of its own (its sidebar dock, its pane group, its layout saved per cluster id), shown with the cluster's colour dot. A tab exists while its session is not `Disconnected`; closing it disconnects (after a confirmation while the cluster has running operations). **ClusterTabs** is the controller that opens, switches, closes and saves them. | `oxikube_workspace::cluster_tab` |
| **Hotbar** | The strip at the window's left edge with a tile (initials, colour dot, state dot) for every connected or favourite cluster, in the user's order. Clicking shows the cluster's tab or connects the favourite. | `oxikube_catalog_ui::hotbar` |
| **Status bar / StatusItem** | The strip under the docks with a left and a right group. A **StatusItem** is a small view (read-only badge, connection state, update notice) registered with a side and a priority; within a side, ascending priority runs left to right. Items notify themselves; the bar never redraws on its own. | `oxikube_workspace::status_bar` |
| **Modal layer / ModalView** | The overlay that hosts one **ModalView** at a time over the whole workspace (a second modal replaces the first). Closes on Escape, a click outside, or the view's `DismissEvent`; traps Tab inside; gives focus back to what had it before. Not gpui-component's dialog layer (the `Root` owns that one). | `oxikube_workspace::modal` |
| **Toast / ToastLayer** | A transient message in the bottom-right corner: a level, a message, optional actions, a dedup key and a timeout (errors stay). The **ToastLayer** shows at most a few at once, queues the rest, and replaces a toast whose key matches instead of stacking it. Not a stored notification; the notifications panel is later work. | `oxikube_workspace::toast` |
| **Main window / Root** | The OS window opened by `oxikube_workspace::window`: native title bar on macOS, client-side decorations on Linux; its root view is `oxikube_ui`'s `Root`, which renders the dialog, sheet and notification layers once above the workspace content. | `oxikube_workspace::window`, `oxikube_ui::root` |
| **Quit guard / running operation** | A **running operation** is something quitting would stop (an exec session, a port-forward, an apply). Features register an *operation provider* that lists theirs; the **quit guard** asks them all on `app::Quit` (and on closing the last window where that quits) and shows a confirm modal instead of quitting while any is running (setting `confirm_quit`). | `oxikube_workspace::session::quit` |
| **UI zoom / reduce-motion** | UI zoom is the `ui_scale` factor (0.5 to 3.0) behind `u(px)` and the window's rem size, changed by `view::ZoomIn`/`ZoomOut`/`ZoomReset` and persisted in `settings.json`. Reduce-motion is the effective flag animations check (`oxikube_ui::motion::reduce_motion`): the `reduce_motion` setting (`system`, `on`, `off`) over the OS preference. | `oxikube_workspace::session`, `oxikube_ui::{size, motion}` |
| **Settings layer** | `default.json → user settings.json → clusters.<id>` overrides. | `oxikube_settings` |
| **ClusterPreset** | A named safety posture a user gives a cluster in one step: **prod** (red, read-only on), **staging** (amber), **dev** (green), **none**. A shortcut, not stored state: applying it writes the same `colour` (and, for prod, `read_only`) fields `settings.json` has, and which preset a cluster "has" is derived from its colour (`ClusterPreset::detect`). A preset never lowers protection. A context name containing `prod` only *suggests* prod (`ClusterPreset::suggest`); nothing applies it silently. | `oxikube_domain::preset`, `oxikube_app::guard::posture` |
| **Read-only badge / ClusterMark** | What a cluster looks like at a glance: its colour dot and, when read-only, a lock, drawn the same on its tab, hotbar entry and status bar item. Colours come from the theme's status tokens. The badge is a convenience; enforcement is the guard's. | `oxikube_workspace::cluster` |
| **ClusterSettings / ClusterPrefs** | The per-cluster settings, resolved field by field through the settings layers: `display_name`, `colour`, `read_only`, `default_namespace`, `terminal_cwd`, `node_shell_image` / `node_shell_pull_secret`, `prometheus` (provider, path or URL, `auth_secret` naming a keychain entry), `accessible_namespaces`, `exec_interactivity`. They are root-level keys, so each is valid at the top of `settings.json` (a default for every cluster) and under `clusters.<id>` (an override). **ClusterSettings** is the `Settings` value in `oxikube_settings`; **ClusterPrefs** is the plain value the app layer understands, and a **ClusterPrefsTable** (global fallback plus a map of the clusters with overrides) is what the binary pushes into the `ClusterSessionManager`. Never holds a secret: the Prometheus token is a keychain reference. | `oxikube_settings::cluster`, `oxikube_ports::cluster_prefs`, `oxikube_app::session::prefs` |
| **Keymap layer** | `default-<os>.json → vim.json (optional flag) → user keymap.json`, merged into one flat list of GPUI `KeyBinding`s; later layers win and `null` unbinds. Zed's `keymap.json` format. | `oxikube_keymap` |
| **Key context** | The name and flags a view sets with `.key_context(..)` (`Table`, `Editing`, `os == macos`); a binding's `context` expression is matched against it. Standard names live in `oxikube_keymap::contexts`. | `oxikube_keymap::context` |
| **Action registry** | The namespaced (`table::SelectNext`) GPUI action names, plus the mapping from an action name to its `Command` (the name *is* the `CommandId`). | `oxikube_keymap::registry` |
| **Theme tokens** | `ThemeTokens`: one theme's colours (interface, editor, terminal, status, VCS, syntax, players), importable from Zed theme-family JSON plus an `oxikube` block for k8s status colours and the cluster-tab palette. Neutral (no gpui-component types); `oxikube_ui` maps it onto gpui-component's `ThemeConfig`. | `oxikube_theme` |
| **Theme selection** | The `theme` setting: a theme name, or `{ mode: system\|light\|dark, light, dark }`; resolved against the system appearance into the `ActiveTheme`. | `oxikube_theme` |
| **AppState** | The typed dependency container of the app (a GPUI global, Zed's pattern): the **ports bundle** (`AppPorts`: `Arc<dyn StatePort>`, the keychain's `SecretStorePort` later) plus accessors over the platform globals (settings store, theme registry, keymap, runtime handle). Built by `bins/oxikube`, the only crate that names adapters; installing it fails unless the runtime, settings, theme and keymap were initialised. `AppState::test(cx)` builds one from testkit fakes. | `oxikube::app_state` (`bins/oxikube`) |
| **Init order** | The documented sequence in which `bins/oxikube` runs each crate's `init(cx)`: logging and panic hook, runtime, assets, settings, theme, keymap, ui, state db (opened in the background), `AppState`, workspace, feature crates, keymap re-bind, open window. Each stage is timed in a tracing span and kept in the `StartupReport`. | `oxikube::startup` |
| **First interactive frame** | The end of start-up: the update that drew and presented the main window's first frame, after which it dispatches input. Timed from the first line of `main` (budget 400 ms) and logged with the per-stage costs and the open network sockets (must be 0). | `oxikube::startup::first_frame` |
| **Startup placeholder** | What the main window shows while its saved layout is read in the background: the workspace's default layout, interactive, marked "Restoring layout…". The restored layout replaces it; a failed read leaves it as the usable default layout. | `oxikube_workspace::window::MainView` |
| **Lazy service** | A service started on first use (`LazyService::ensure_init`) instead of by an `init(cx)`: extension host, discovery, Prometheus detection, agent registry, update checker. | `oxikube_runtime::lazy` |
| **Crash report** | The local, redacted file the panic hook writes (`<data dir>/crashes/crash-<time>-<pid>.log`: version, OS, thread, location, message, backtrace). Never uploaded by the app; the opt-in uploader is `oxikube_crash`. | `oxikube_logging::crash` |
| **Fake / fixture / builder** | A scripted in-memory implementation of a port, a realistic JSON manifest loaded as a `Resource`, and a fluent `Resource` builder (`pod().running().build()`). | `oxikube_testkit::{fakes, fixtures, builders}` |
