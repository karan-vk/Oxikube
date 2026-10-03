# Domain glossary (CONTEXT.md)

Shared vocabulary. Type names in code match these terms; if you need a new term, add it here first.

| Term | Meaning | Lives in |
|---|---|---|
| **ClusterId** | Stable identifier for a cluster entry in the catalog (hash of kubeconfig source + context name). | `oxikube_domain::ids` |
| **ContextName** | A kubeconfig context name. | `oxikube_domain::ids` |
| **ClusterSession** | A connected cluster: client pool, discovery, feeds, namespace selection, read-only flag, colour. One workspace tab per session. | `oxikube_app::session` |
| **ClusterSessionState** | `Disconnected → Connecting → (AuthRequired) → Ready ↔ Degraded → Error`. | `oxikube_domain::session` |
| **Gvk / Gvr** | Group-Version-Kind / Group-Version-Resource. | `oxikube_domain::ids` |
| **ResourceRef** | `{cluster, gvk, namespace, name}` — the address of one object. | `oxikube_domain::ids` |
| **Resource** | Thin model: `ObjectMeta` + raw `serde_json::Value`. The domain never depends on k8s-openapi. | `oxikube_domain::resource` |
| **View-model** | Typed projection built from `Resource` JSON for a core kind (`PodSummary`, `NodeSummary`, `WorkloadSummary`, …). | `oxikube_domain::view` |
| **Feed** | A live stream of `Delta`s for one (cluster, gvk, scope): reflector, metadata-only, or Table API. | `oxikube_kube::feed`, `oxikube_ports` |
| **ResourceStore** | App-side cache over feeds with sort/filter/index and ref-counted subscriptions. | `oxikube_app::store` |
| **WatchScope / NamespaceSelection** | `All` or a `Set` of namespaces; decides whether feeds are cluster- or namespace-scoped. | `oxikube_domain::session` |
| **ColumnProvider** | Produces table columns + cells for a kind (core columns, Table API columns, metrics columns). | `oxikube_app::columns` |
| **KindSpec** | Per-kind registration: columns, detail renderer, actions, templates, sidebar section. | `oxikube_resources_ui::kinds` |
| **Command** | A user/agent action with metadata (`CommandMeta`: title, scope, mutating, confirm tier, capabilities). Dispatched through the **CommandBus**. | `oxikube_domain::command`, `oxikube_app::command_bus` |
| **Capability** | Bitflags of what a session/backend can do (mutate, exec, logs, portforward, helm, argo…). | `oxikube_domain::command` |
| **MutationGuard** | The single pipeline every mutation passes: read-only check → confirmation tier → dry-run → execute → audit. | `oxikube_app::mutation` |
| **Risk / ConfirmTier** | Stories say "confirm low / medium / high". Mapping: **Low** → `Simple` (one-click confirm; may be skipped per session by setting), **Medium** → `Simple` naming the target and cluster, **High** → `TypeName`, **Irreversible** → `TypeName` + mandatory server dry-run diff. Reads never confirm. Read-only clusters reject every tier with `ErrorKind::ReadOnly`. | `oxikube_domain::safety` |
| **AuditRecord** | Who/what/where/when/outcome/initiator for a mutation; stored in SQLite. | `oxikube_domain::audit` |
| **Initiator** | `Ui / Command / Agent / Plugin` — who asked for a mutation. | `oxikube_domain::safety` |
| **Item / Panel / Pane / Dock** | Workspace shell concepts (Zed design): tab content, dockable side panel, split container, edge dock. | `oxikube_workspace` |
| **TerminalBackend** | Byte-stream source for the terminal element: local PTY, kube exec/attach, display-only. The trait lives in the ports layer so adapters can implement it; the element that paints it lives in `oxikube_terminal`. | trait `oxikube_ports::exec`; impls in `oxikube_terminal` (local PTY, display-only), `oxikube_kube` (exec/attach), `oxikube_argocd` (terminal WS) |
| **Integration** | Optional capability pack (Argo CD, Flux) detected per cluster; contributes sidebar, commands, tools, context providers, settings. | `oxikube_ports::integration`, `oxikube_app::integrations` |
| **ArgoBackend** | CRD-direct / Server REST / Core helper implementations behind one trait with capability flags. | `oxikube_argocd` |
| **Tool / ToolDef** | An MCP-shaped capability (name, JSON schema, risk) in the **ToolRegistry**; mutating tools are permission-gated. | `oxikube_domain::agent`, `oxikube_app::tools` |
| **ContextProvider / Mention** | Resolves `@kind/ns/name`, `@logs/...`, `@events/...` into **ContentBlock**s with size budgets. | `oxikube_app::context` |
| **AgentSession / AgentThread** | An ACP session with a hosted agent and its thread of entries (messages, thoughts, tool calls, plan, permissions). | `oxikube_domain::agent::thread`, `oxikube_app::agent` |
| **Extension** | A WASM component (WIT world `oxikube:extension`) contributing themes, commands, MCP servers; never UI. | `oxikube_extension_api`, `oxikube_extension_host` |
| **Settings layer** | `default.json → user settings.json → clusters.<id>` overrides. | `oxikube_settings` |
| **Theme tokens** | Our colour/spacing tokens, importable from Zed theme-family JSON plus an `oxikube` block for k8s status colours. | `oxikube_theme` |
