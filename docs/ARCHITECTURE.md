# Oxikube architecture

Oxikube is a native Kubernetes desktop client written in Rust on GPUI (Zed's UI framework).
It is organised as a hexagonal (ports & adapters) workspace so that the domain and the
use-cases never know about kube-rs, GPUI, SQLite, Argo CD or ACP. Adapters implement ports;
the binary wires them together.

## Layers and dependency direction

```
domain  ←  ports  ←  app  ←  (ui, bins)
             ↑
          adapters        (implement ports; never depend on app or ui)
platform                  (settings / keymap / theme / runtime / extension host; → domain, ports)
bins/oxikube              (wires adapters into app, mounts ui)
```

| Layer (`crates/<layer>/`) | May depend on (internal) | Must not depend on (external) |
|---|---|---|
| `domain` | nothing | gpui*, kube*, k8s-openapi, tokio, reqwest, rusqlite, wasmtime |
| `ports` | domain | gpui*, kube*, k8s-openapi, reqwest, rusqlite, wasmtime |
| `app` | domain, ports | gpui*, kube*, k8s-openapi, reqwest, rusqlite, wasmtime, agent-client-protocol, rmcp |
| `adapters` | domain, ports | gpui* |
| `platform` | domain, ports, platform | kube*, k8s-openapi, gpui-component |
| `ui` | domain, ports, app, platform, ui | kube*, k8s-openapi; `gpui-component` only inside `oxikube_ui` |
| `testing` | domain, ports | gpui-component |
| `bins`, `xtask` | anything | – |

`cargo xtask lint-deps` parses `cargo metadata` and fails CI on any violation. Dev-dependencies
are exempt so tests can use `oxikube_testkit` freely.

Why so strict: multiple agent teams work in parallel. The layer rules are what let a team
replace an adapter (kube-rs → something else, gpui-component → in-house) or add an integration
(Argo CD, Flux) without touching the core, and let the domain/app layers be tested with fakes
in milliseconds.

## Crate map

See `docs/PLAN.md` ("Workspace layout") for the one-line responsibility of every crate, and each
crate's `README.md` for its allowed dependencies. Highlights:

- `oxikube_domain` — ids, the thin `Resource { meta, json }` model, view-models, `Command` /
  `Capability` vocabulary, `Quantity`, `Age`, error taxonomy, safety types, redaction.
- `oxikube_ports` — every async, object-safe trait (`ResourcePort`, `LogPort`, `ExecPort`,
  `StatePort`, `IntegrationPort`, `ToolPort`, `AgentPort`, …).
- `oxikube_app` — services: `ClusterSessionManager`, `ResourceStore`, `CommandBus`,
  `MutationGuard`, `LogService`, `PortForwardManager`, `IntegrationRegistry`, `ToolRegistry`,
  `ContextRegistry`, `AgentSessionManager`. No gpui, no kube.
- `oxikube_kube` — the kube-rs adapter (connection, discovery, reflectors, Table API feed,
  mutations, subresources, logs, exec, port-forward, metrics, events).
- `oxikube_ui` — the only crate that imports `gpui-component`; exposes tokens and curated
  components to every view.
- `oxikube_workspace` — Zed-style Item / Panel / Pane / Dock shell with persistence.

## Cross-layer wiring (ports + injection)

Adapters never depend on other adapters, on `oxikube_app` or on UI crates. Platform crates never
depend on each other's consumers. When a component needs a capability that lives in another
layer, **define a narrow port in `oxikube_ports` and inject the implementation from
`bins/oxikube`**. Examples that the plan relies on:

| Needs | Port (in `oxikube_ports`) | Implemented by | Consumed by |
|---|---|---|---|
| MCP server exposing app tools | `ToolPort` (registry handle) | `oxikube_app::ToolRegistry` | `oxikube_mcp` |
| ACP `terminal/*` passthrough | `TerminalHostPort` | `oxikube_terminal` | `oxikube_acp` |
| Argo backends reading the cluster | `ResourcePort`, `PortForwardPort`, `ExecPort` | `oxikube_kube` | `oxikube_argocd` |
| Extensions contributing themes/commands/MCP servers | `ThemeSinkPort`, `CommandSinkPort`, `ContextServerSinkPort` | `oxikube_theme`, `oxikube_app`, `oxikube_mcp` | `oxikube_extension_host` |
| App reading user settings (aliases, budgets) | plain values pushed in at init / on change | `oxikube_settings` (via bins) | `oxikube_app` |
| Terminal byte streams | `TerminalBackend` (in `oxikube_ports::exec`) | `oxikube_terminal`, `oxikube_kube`, `oxikube_argocd` | `oxikube_terminal` element |

If a story's crate list implies a forbidden edge, follow this table and say so in the PR; do not
weaken `cargo xtask lint-deps`.

## Key runtime rules

1. **No network or blocking work on the UI thread.** Kubernetes work runs on the shared Tokio
   runtime via `oxikube_runtime::spawn_kube` (abort-on-drop); results are posted back as GPUI
   tasks and `cx.notify()` calls are coalesced to frame cadence.
2. **Every mutation goes through `MutationGuard`** (read-only mode → confirmation tier →
   server dry-run → execute → audit). UI, keymap actions, command palette and MCP tools all
   dispatch `Command`s through the `CommandBus`; nothing calls a mutating port directly.
3. **Every user-facing action is a `Command`** with metadata (title, scope, mutating, confirm
   tier, capabilities) and registers an MCP tool stub, so agents get the same surface as users.
4. **Secrets never touch disk** except through `SecretStorePort` (OS keychain). Logs, audit
   records and crash reports are redacted.
5. **GPUI pins move together.** `gpui-pre-*` and `gpui-component`/`gpui-base`/`gpui-kit-assets`
   are exact-pinned and bumped in one PR; `cargo xtask check-gpui-pin` enforces alignment.
7. **It must feel as smooth as Zed.** `docs/PERFORMANCE.md` holds the numeric budgets
   (frame p95 ≤ 8 ms under churn, input ≤ 1 frame, cold start ≤ 400 ms, idle CPU < 1 %) and the
   rules that keep us inside them; ADR 0013 makes them release-gating.
6. **Zed code is copied, never depended on.** Zed's app crates are GPL-3.0-or-later like us, so
   selected modules (settings store, keymap, theme loader, picker, terminal element) may be
   vendored with a GPL header and an entry in `THIRD_PARTY_NOTICES.md`.

## Data flow for a resource table

```
kube API ──watch──▶ oxikube_kube::feed (reflector / metadata / Table) ──Delta batches──▶
oxikube_app::ResourceStore (cache, sort, filter, index) ──subscribe──▶
oxikube_resources_ui::ResourceTable (uniform_list rows via oxikube_ui::Table) ──▶ GPUI
```

Core kinds use typed/metadata reflectors plus our own column definitions; CRDs and unknown kinds
use the server-side Table API (kubectl-identical columns incl. `additionalPrinterColumns`).

## Integrations

Optional integrations (Argo CD first; Flux later) implement `IntegrationPort`: detect → sidebar
section, commands, MCP tools, `@`-mention context providers, settings section. Their code lives in
`crates/adapters/oxikube_<name>` and `crates/ui/oxikube_<name>_ui`, never sprinkled through core.

## Agents

Oxikube hosts external agents over ACP (`oxikube_acp`, `oxikube_agent_ui`) and exposes its own
cluster tools to them over MCP (`oxikube_mcp` serving `oxikube_app::ToolRegistry`). Agents can
also drive the UI through `app.*` tools that dispatch `Command`s. Agent-proposed manifests open
as editor diffs and apply only through `MutationGuard`.

## Decision records

See `docs/adr/`. ADRs are the contract; change an ADR before changing an architectural rule.
