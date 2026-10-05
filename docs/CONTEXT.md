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
| **ErrorKind** | `Auth`, `Forbidden`, `NotFound`, `Conflict`, `Network`, `Timeout`, `Validation`, `Unsupported`, `Internal`. Adapter mapping rules are in `docs/ARCHITECTURE.md`. | `oxikube_domain::error` |
| **Redaction** | Removing tokens and Secret data from text before it is logged, audited or put in an error. `oxikube_domain::redact` holds the pure scrubber (`redact(&str) -> Cow<str>`, `Redacted<T>`); adapters redact before building an error and `oxikube_logging` scrubs every log line with it. | `oxikube_domain::redact` |

## Sessions and state

| Term | Meaning | Lives in |
|---|---|---|
| **ClusterSession** | A connected cluster: client pool, discovery, feeds, namespace selection, read-only flag, colour. One workspace tab per session. | `oxikube_app::session` *(planned)* |
| **ClusterSessionState** | `Disconnected → Connecting → (AuthRequired) → Ready ↔ Degraded → Error`. Advanced by `SessionEvent`s; `SessionPhase` is the payload-free discriminant; a bad pair is an `InvalidTransition`. | `oxikube_domain::session` |
| **NamespaceSelection** | `All` or a `Set` of namespaces chosen in the UI. `NamespaceFavourites` is the user's pinned list. | `oxikube_domain::session` |
| **WatchScope** | Derived from a `NamespaceSelection` and a kind's `Scope`: `Cluster` or `Namespaces`; decides whether feeds are cluster- or namespace-scoped. | `oxikube_domain::session` |
| **Feed** | A live stream of `DeltaBatch`es for one (cluster, gvk, scope): reflector, metadata-only, or Table API. Typed as `WatchFeed`. | `oxikube_ports::feed`; producers in `oxikube_kube::feed` (reflector and metadata-only variant: `ReflectorFeed`, E04-S02/S03) |
| **FeedState** | Health of one feed: `Warming` (listing or relisting), `Live`, `Retrying` (backing off after a watch error), `Stopped`. Lets the session go `Degraded` while a feed retries. | `oxikube_kube::feed` |
| **Delta / DeltaBatch** | One change in a feed (`Applied`, `Deleted`, `Restarted`) and a coalesced batch of them. Lives in ports, not the domain: it is transport. | `oxikube_ports::feed` |
| **Table / TableFeed** | Server-side Table API data (kubectl-identical columns incl. CRD printer columns) as `TableColumn`s and `TableRow`s. | `oxikube_ports::table`; producer `oxikube_kube::table` |
| **TableSource** | Whether a `Table`'s columns are the server's (`Server`) or the adapter's plain-JSON fallback (`Objects`, the server ignored the Table `Accept` header). | `oxikube_ports::table` |
| **ResourceStore** | App-side cache over feeds with sort/filter/index and ref-counted subscriptions. | `oxikube_app::store` *(planned)* |
| **ColumnProvider** | Produces table columns + cells for a kind (core columns, Table API columns, metrics columns). | `oxikube_app::columns` *(planned)* |
| **KindSpec** | Per-kind registration: columns, detail renderer, actions, templates, sidebar section. | `oxikube_resources_ui::kinds` *(planned)* |

## Commands and safety

| Term | Meaning | Lives in |
|---|---|---|
| **Command** | A user/agent action: the `Command` payload enum, identified by a `CommandId` (`workload::Scale`) and described by `CommandMeta` (title, `CommandScope`, mutating, confirm tier, capabilities). Dispatched through the **CommandBus**. The id is also the keymap action and the MCP tool name. | `oxikube_domain::command`; bus in `oxikube_app::command_bus` *(planned)* |
| **Capability** | A flag for what a session/backend can do (mutate, exec, logs, portforward, helm, argo…). `Capabilities` is the set. | `oxikube_domain::command` |
| **MutationGuard** | The single pipeline every mutation passes: read-only check → confirmation tier → dry-run → execute → audit. It alone holds a `ResourceWriter`. | `oxikube_app::mutation` *(planned)* |
| **Risk / ConfirmTier** | Stories say "confirm low / medium / high". Mapping: **Low** → `Simple` (one-click confirm; may be skipped per session by setting), **Medium** → `Simple` naming the target and cluster, **High** → `TypeName`, **Irreversible** → `TypeName` + mandatory server dry-run diff. Reads never confirm. A read-only cluster refuses every tier before any request and records `AuditOutcome::Denied`; `ErrorKind` has no `ReadOnly` variant, so the error kind the guard returns is decided with the guard (E19). | `oxikube_domain::safety` |
| **Initiator** | `Ui / Command / Agent / Plugin` — who asked for a mutation. Defined in `safety`, re-exported from `audit`. | `oxikube_domain::safety` |
| **AuditRecord** | Who/what/where/when/outcome/initiator for a mutation (`AuditOutcome`: Succeeded, Failed, Denied, Cancelled). Never holds request bodies. Stored in SQLite through `StatePort`. | `oxikube_domain::audit` |

## Ports

A **port** is an async, object-safe trait in `oxikube_ports` that the app depends on and an adapter implements. Every fallible method returns `OxiResult`. Each port's module docs name its adapter; each has a `Fake*` in `oxikube_testkit::fakes`.

| Term | Meaning | Lives in |
|---|---|---|
| **ResourcePort** | `ResourceReader` + `ResourceWriter`. Only `MutationGuard` holds the writer half. | `oxikube_ports::resource` |
| **Forward** | One port-forward: a `ForwardSpec` (a pod or service `ResourceRef`, a remote `ForwardPort`, a local port and bind address, loopback by default) and its `ForwardStatus` (`Starting`, `Listening`, `TargetGone`, `Error`, `Stopped`). Run by the kube adapter, owned by `PortForwardManager` (E15). | `oxikube_domain::portforward`; adapter in `oxikube_kube::remote::portforward` |
| **DiscoveryPort / LogPort / ExecPort / PortForwardPort / TableFeedPort** | Data-plane access to a connected cluster. | `oxikube_ports::{discovery, log, exec, portforward, table}` |
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
| **Item / Panel / Pane / Dock** | Workspace shell concepts (Zed design): tab content, dockable side panel, split container, edge dock. | `oxikube_workspace` |
| **Settings layer** | `default.json → user settings.json → clusters.<id>` overrides. | `oxikube_settings` |
| **Theme tokens** | Our colour/spacing tokens, importable from Zed theme-family JSON plus an `oxikube` block for k8s status colours. | `oxikube_theme` |
| **Fake / fixture / builder** | A scripted in-memory implementation of a port, a realistic JSON manifest loaded as a `Resource`, and a fluent `Resource` builder (`pod().running().build()`). | `oxikube_testkit::{fakes, fixtures, builders}` |
