# Architecture rules

The canonical source is `docs/ARCHITECTURE.md`; this file is the working summary with the
rules the lint enforces and the conventions reviewers check.

## Layers and allowed dependencies

| Layer | Crates | May depend on (internal) | Banned external |
|---|---|---|---|
| domain | `oxikube_domain` | nothing | `gpui*`, `kube*`, `k8s-openapi`, `tokio` (runtime), `reqwest`, any I/O |
| ports | `oxikube_ports` | domain | `gpui*`, `kube*`, `k8s-openapi` |
| app | `oxikube_app` | domain, ports | `gpui*`, `kube*`, `gpui_component`, any adapter crate |
| adapters | `oxikube_kube`, `oxikube_describe`, `oxikube_helm`, `oxikube_prometheus`, `oxikube_cloud`, `oxikube_argocd`, `oxikube_state_sqlite`, `oxikube_keychain`, `oxikube_notify_os`, `oxikube_updater`, `oxikube_crash`, `oxikube_acp`, `oxikube_mcp` | domain, ports (+ `oxikube_logging` for redaction) | `gpui*`, `gpui_component`, app, ui crates |
| platform | `oxikube_runtime`, `oxikube_settings`, `oxikube_keymap`, `oxikube_theme`, `oxikube_assets`, `oxikube_extension_api`, `oxikube_extension_host`, `oxikube_logging` | domain, ports (gpui allowed) | `kube*`, app, ui, adapters (`oxikube_extension_host` may use `oxikube_runtime`) |
| ui | `oxikube_ui`, `oxikube_workspace`, `oxikube_palette`, `oxikube_catalog_ui`, `oxikube_resources_ui`, `oxikube_overview_ui`, `oxikube_logs_ui`, `oxikube_terminal`, `oxikube_editor`, `oxikube_portforward_ui`, `oxikube_events_ui`, `oxikube_helm_ui`, `oxikube_rbac_ui`, `oxikube_settings_ui`, `oxikube_extensions_ui`, `oxikube_argocd_ui`, `oxikube_agent_ui` | domain, ports, app, platform, `oxikube_ui` | adapters; `gpui_component` (only `oxikube_ui` may import it) |
| bins | `bins/oxikube` | everything | — |
| testing | `oxikube_testkit` | domain, ports, (gpui test-support) | — |
| tooling | `xtask` | none | — |

`cargo xtask lint-deps` reads `cargo metadata`, classifies crates by path
(`crates/<layer>/...`), and fails on any edge not in the table. It also bans the listed
external crates per layer. If the lint is wrong, fix the lint in the same PR with a test
in `xtask/src/lint_deps.rs`; do not add an `allow`.

## What goes where (decision table)

| You are adding... | Put it in |
|---|---|
| A new Kubernetes concept type (view-model, id, status enum) | `oxikube_domain` (module per concept; JSON-driven, no k8s-openapi) |
| A new capability the app needs from the outside world | a trait in `oxikube_ports` + a fake in `oxikube_testkit` |
| Orchestration, caching, policy, cross-port logic | a service in `oxikube_app` |
| Code that talks to kube-rs, a CLI, HTTP, SQLite, keychain | the matching adapter crate |
| A user-visible action | a `Command` variant in `oxikube_domain::command`, a handler in `oxikube_app` registered from the owning crate's `init`, a tool stub in the ToolRegistry, then UI |
| A view/panel/tab | the ui crate for that feature; shared widgets in `oxikube_ui` |
| A gpui-component usage | `oxikube_ui` only, re-exported through a curated API |
| A setting | content struct in the owning crate, default in `default.json`, schema via `xtask gen-settings-schema` |
| A keybinding | per-OS default keymap JSON in `oxikube_keymap`, with a key context |
| A theme colour | `ThemeTokens` in `oxikube_theme` (and the `oxikube` block for k8s status colours) |
| An MCP tool | `ToolDef` + invoker in `oxikube_app::tools::<ns>`; integrations register groups |
| An optional integration (Argo, Flux...) | an `IntegrationPort` impl in its adapter crate + its own ui crate; nothing in core |

## Naming

- Crates: `oxikube_<noun>`; ui crates end in `_ui` except the four "big widgets"
  (`oxikube_workspace`, `oxikube_palette`, `oxikube_terminal`, `oxikube_editor`).
- Ports: `<Thing>Port` traits, methods are verbs, errors are `OxiError`.
- Services: `<Thing>Service` or `<Thing>Manager`; a service owns state, a port does not.
- Commands: `namespace::Verb` ids (`pod::Delete`, `workload::Scale`, `cluster::ToggleReadOnly`).
- Tools: `k8s.*`, `helm.*`, `argo.*`, `app.*`, `ext.<id>.*`.
- Modules mirror the story's "Crates/modules" column; one file per concept.
- Each crate exposes `pub fn init(cx: &mut App)` (ui/platform) or a constructor
  (app/adapters); `bins/oxikube` calls them in a documented order.

## Dependency pins that matter

- `gpui = { package = "gpui-pre", version = "=<x>" }`, `gpui_platform = { package = "gpui-pre-platform", ... features = ["font-kit","x11","wayland","runtime_shaders"] }`, `gpui-component = "=<y>"` paired with it. Bump together.
- `kube` with `default-features = false` and explicit features; one TLS provider (`ring`).
- `k8s-openapi` feature `latest`; `jiff` not `chrono`; `serde-saphyr` for YAML.
- `agent-client-protocol = "=2.2.0"`.
- All versions live in `[workspace.dependencies]`; crates use `foo.workspace = true`.
