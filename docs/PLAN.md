# Oxikube — Lens/OpenLens alternative in Rust + GPUI

## Context

Lens/OpenLens is Electron + React with a login wall, high memory, and a frozen upstream; Freelens keeps it alive but inherits the stack. k9s is keyboard-perfect but terminal-bound. The user wants a native, Zed-feeling Kubernetes desktop client in Rust/GPUI, structured as a large multi-crate workspace following DRY/KISS/hexagonal principles, built by multiple agent teams working from GitHub epics and stories, with Zed-style platform features (terminal, editor, settings, themes, WASM extensions), an optional native Argo CD integration, and ACP-hosted coding agents (Claude Code, Codex, Antigravity, Gemini) as the final phase but designed-for from day one.

This turn's deliverable: repo `karan-vk/Oxikube` created and scaffolded, a GitHub Project with fields/views, every epic + story created as issues/sub-issues, and an in-repo contributor skill that all agent teams follow.

## Decisions (grilling rounds, 51 questions) — the contract

| # | Decision |
|---|----------|
| Q1 | Platforms: macOS + Linux from day one; Windows is a late epic |
| Q2/Q50 | Public repo `karan-vk/Oxikube`. **License: GPL-3.0-or-later for everything incl. extension API** (Zed's model; changed mid-session from Apache/MIT). kdash (MIT), gpui-component (Apache-2.0), deskribe (Apache-2.0 + k8s NOTICE) stay compatible; attribution in THIRD_PARTY_NOTICES.md |
| Q3 | Many clusters connected at once, one workspace tab per cluster, cluster catalog as home |
| Q4 | Milestones: Foundation → core k8s (browse/logs/exec/edit) → full Lens parity → Zed-style platform → Argo CD → ACP |
| Q5 | Plugins: Zed-identical surface (WASM/wasmtime + WIT): themes, commands, MCP servers. NO UI hooks |
| Q6 | Editor: purpose-built manifest editor (YAML/JSON, tree-sitter, OpenAPI schema validation, diff vs live, dry-run, apply) |
| Q7 | ACP: Oxikube is an ACP client hosting agents in a panel; exposes an MCP server of cluster tools (gated mutations); agent actions are first-class commands via a command bus. No headless MCP CLI |
| Q8 | Domain owns a thin Resource model (GVK + metadata + raw JSON) + typed view-models; k8s-openapi only inside the kube adapter |
| Q9 | GitHub: epics as issues + native sub-issues (stories) with acceptance criteria |
| Q10 | Project fields: Status, Phase, Area, Priority, Size, Team; views: Board by Status, Roadmap by Phase, Table by Area |
| Q11 | CI: fmt/clippy/unit on Linux+macOS per PR; kind integration on Linux; nightly full matrix; cargo-deny |
| Q12 | Release: cargo-dist → GitHub Releases (dmg, tar.gz, deb, AppImage); notarization gated on a Developer ID secret |
| Q13 | Metrics: metrics-server always + pluggable Prometheus adapter (auto-detect) for history graphs |
| Q14 | Themes: Zed-compatible theme-family JSON import + oxikube block for k8s status colours; own settings.json/keymap.json with Zed-style layering (default → user → per-cluster) + GUI settings page |
| Q15/28 | Terminal: Zed-like movable tab item running any command, env pre-set per cluster; exec/attach/node-shell/debug open as terminal tabs via a TerminalBackend trait (local PTY + kube-ws) |
| Q16 | Auto-update (GitHub Releases) + opt-in anonymous crash reporting (Sentry-style) behind a port |
| Q17 | Claude Code teams primarily; AGENTS.md mirrors the skill for Codex/Gemini; story = branch = PR; CI green + reviewer agent; squash merge |
| Q18 | Crate layout grouped by hexagonal layer (see Architecture); xtask enforces dependency direction |
| Q19/21/25 | Cluster discovery: kubeconfig sources (hot reload) + EKS/GKE/AKS via installed CLIs (aws/gcloud/az), provider hidden when CLI missing |
| Q22 | Namespaces: Lens-style All + multi-select + favourites, remembered per cluster; watcher scope follows selection |
| Q23 | k8s-openapi newest feature; support last 4 upstream minors; discovery for the rest |
| Q24 | Persistence: SQLite (rusqlite) behind a StatePort |
| Q26 | Logs: single + stern-style multi-pod, regex search/highlight, save, JSON structured parsing, kubectl-in-terminal fallback; logs easily sent to / queried by the ACP agent |
| Q27 | Helm: HelmPort with native release-Secret reader + helm-CLI adapter for mutations; hidden when helm missing |
| Q29 | Navigation: Zed command palette + k9s ':' jump with aliases + '/' filter + rebindable keymap contexts + optional vim keymap |
| Q30/48 | ACP presets: Claude Code (claude-acp npx), Codex (codex-acp npx), Antigravity (official binary adapter), Gemini (npx --acp), custom agent_servers; registry.json fetched at runtime (schema-validated, cached) with built-in presets as fallback |
| Q31 | Safety: per-cluster read-only mode + colour-coded tabs; confirm dialogs ('type the name' for ns/node/PV); server dry-run diff before apply; local audit log |
| Q32 | Detail: structured per-kind panel + Describe tab + YAML tab + Events tab; generic panel for unknown kinds/CRDs |
| Q33 | Create from templates, apply pasted/opened multi-doc YAML, bulk multi-select ops, apply directory/kustomize output, copy resource between clusters |
| Q34 | Persistent port-forward manager (auto-restart, open-in-browser, favourites restored) |
| Q35 | Cluster-wide events view + per-resource events; notification centre; opt-in OS notifications; cluster overview dashboard |
| Q36 | Agent context: @-mentions via ContextProviders; 'Send to agent' everywhere; ambient context; agent-proposed YAML opens in editor diff gated by permission |
| Q37/43/44 | Argo CD: ArgoBackend trait with CRD-direct (kube + kopium types), Server REST (hand-written reqwest, SSE/NDJSON, cookie-auth terminal WS), Core helper (spawn `argocd admin dashboard --port N`); capability-gated UI; auth = token entry + import `~/.config/argocd/config` + OS keychain + native SSO PKCE loopback (openidconnect) |
| Q38 | Argo Rollouts in the Argo epic (CRD-direct merge patches) |
| Q39/40 | Generic IntegrationPort (detect → sidebar, commands, MCP tools, context providers, settings); Argo CD first; Flux follow-up |
| Q41/42/45/51 | GPUI via `gpui-pre` pinned exactly to gpui-component's pin (=0.3.7 ↔ zed@1a28cff), gpui-component (Apache-2.0) behind thin `oxikube_ui` wrapper; **vendor/copy Zed GPL code selectively** (settings store, keymap, theme loader, terminal element, picker) with license headers; never git-depend on Zed crates; xtask checks pin alignment; documented fallback = Zed git rev + forked gpui-kit |
| Q46 | Table data: hybrid — typed/metadata reflectors + own column defs for core kinds; server Table API (hand-rolled Accept header, list+watch) for CRDs/unknown kinds; one ColumnProvider trait |
| Q47 | Plugins runtime: wasmtime component model + WIT `since_vX` worlds, extension.toml capability grants, epoch interruption; SDK crate `oxikube_extension_api` |
| Q52 | **Smoothness is a requirement (user, mid-session): the app must feel as smooth as Zed.** Numeric budgets in docs/PERFORMANCE.md + ADR 0013; hot-path PRs report `--perf` numbers; nightly perf regression gate |
| Q49 | This turn's execution scope: full compiling skeleton (all crates empty, xtask lint, CI, cargo-deny, rust-toolchain, docs/ADRs, skill, AGENTS.md/CLAUDE.md) |

Technical defaults fixed by me (conventional, no user decision needed): Rust edition 2024, `rust-toolchain.toml` = stable 1.99 (wasmtime 49 needs ≥1.96); kube 4.2 with `default-features=false` + `client, config, runtime, derive, ws, oauth, oidc, gzip, http-proxy, socks5, rustls-tls, ring` (one TLS provider, no C toolchain); k8s-openapi 0.28 `latest`; jiff (not chrono); serde-saphyr for YAML (serde_yml is RUSTSEC-unsound); `deskribe` behind DescribePort with kubectl fallback; `k8s-metrics` behind MetricsPort with internal fallback types; `gpui_tokio` bridge with abort-on-drop `spawn_kube`; `alacritty_terminal` 0.26 from crates.io; `agent-client-protocol =2.2.0` pinned; `keyring` for secrets; `rusqlite` bundled; `similar` for diffs; `granit-parser` for spanned YAML in the editor; `nucleo-matcher` for fuzzy; `notify` for file watching; `cargo-deny` with advisories + license allow-list (GPL-3.0-or-later, Apache-2.0, MIT, BSD, ISC, MPL-2.0, CC0, Unicode).

## Research facts that shape the plan (full reports in conversation; summarised)
- Freelens/k9s feature inventories captured in scratchpad `research/report-features.md` (779 lines) → source of the resource catalog and per-kind actions.
- kube-rs 4.2: no Table API (hand-roll like sofka/kubetui), no metrics types (use k8s-metrics), watcher Config with page_size/StreamingList, SSA + dry-run, subresources incl. evict/ephemeral containers/scale, exec/attach/portforward via `ws`.
- kdash (MIT): reuse only small pure functions — tolerant multi-path KUBECONFIG loader (`network/mod.rs:243-333`), cronjob trigger (`mod.rs:844-909`), merge-patch builders (`mod.rs:119-169`), log reconnect/dedup logic (`stream.rs:126-307`), troubleshoot rules (`app/troubleshoot/*`). Do NOT copy its polling architecture, quantity parsing, or pod status derivation (buggy init branch).
- gpui-component 0.7.0: table on uniform_list, dock with serialisable PanelState, editor (ropey + tree-sitter + DiagnosticSet), charts, markdown, command palette, theme JSON (syntax section accepts Zed `style`). Pitfalls: weekly API churn, exact pin coupling, Root overlay layers, resizable panels in absolute px.
- Zed (GPL, copyable now): per-crate `init(cx)`, AppState global, Item/Panel/Dock/Pane, PickerDelegate, Settings/SettingsStore layering with comment-preserving JSON edits + schemars, keymap JSON contexts, theme-family JSON schema v0.2.0, extension.toml + WIT `since_vX` + capability grants + epoch interruption, ACP client via `agent-client-protocol` with session/update, request_permission, fs/*, terminal/*, elicitation.
- Kubyl/Periscope lessons: pin gpui-pre + gpui-component together; `render_to_image` needs test-support on both gpui and gpui_platform; GPUI tests non-deterministic when OS threads (notify) wake tasks; a Task must not drop itself; wrap sizes for zoom; Windows/Linux untested interactively by anyone — plan real smoke tests; GPUI a11y immature.
- Argo CD v3.5.3: 82 REST paths; `--core` is a local in-process server (port-forwards repo-server + Redis), not CRD-only; SSO is PKCE loopback; terminal WS is cookie-authed; no maintained Rust client; Rollouts fully CRD-direct.
- ACP registry: 41 agents; antigravity-acp 1.3.0 is an official Google binary adapter; claude-acp/antigravity licensed "proprietary" → fetch at runtime, never bundle.

## Architecture

### Hexagonal layers and dependency direction (enforced by `cargo xtask lint-deps`)
```
domain  ← ports ← app ← (ui, bins)
            ↑
         adapters  (implement ports; never depend on app or ui)
platform (settings/keymap/theme/extension host/runtime) → domain, ports only; used by ui + bins
bins/oxikube wires adapters into app and mounts ui
```
Rules: `oxikube_domain` has no internal deps and no I/O crates; `oxikube_ports` depends only on domain; `oxikube_app` on domain+ports (NO gpui, NO kube); adapters on domain+ports (+ their external SDKs); `platform/*` on domain+ports (+gpui where unavoidable); `ui/*` on domain, ports, app, platform, `oxikube_ui`; `bins/oxikube` on everything. The lint parses `cargo metadata` and fails CI on violations, and bans `gpui`/`kube` from the wrong layers.

### Workspace layout (every crate created empty in this turn, with README.md stating its responsibility and allowed deps)
```
Oxikube/
├── Cargo.toml                 workspace: resolver 2, edition 2024, [workspace.dependencies], [workspace.lints] (Zed-style: todo/dbg deny), profiles (dev opt-level=3 for proc-macros, tree-sitter, taffy, resvg, wasmtime; release lto=thin)
├── rust-toolchain.toml  deny.toml  .cargo/config.toml  LICENSE-GPL  THIRD_PARTY_NOTICES.md  CODEOWNERS
├── xtask/                     lint-deps, check-gpui-pin, gen-settings-schema, gen-wit-bindings, kind-up/down, load-pods (churn), screenshot
├── bins/oxikube/              binary; init order like Zed main.rs; AppState global; panic/crash handler
├── crates/domain/oxikube_domain          ids (ClusterId, ContextName), Gvk/Gvr, ResourceRef, Resource{meta, json}, NamespaceSelection, view-models (PodSummary, NodeSummary, WorkloadSummary…), LogLine, Event, MetricsSample, Quantity parser, Age fmt, Command enum (command bus), Capability flags, Audit record, integration ids, agent context blocks
├── crates/ports/oxikube_ports            traits: ClusterSourcePort, CloudDiscoveryPort, ResourcePort (list/watch/get/create/replace/patch/apply/dry_run/delete/delete_collection/subresources), TableFeedPort, DiscoveryPort, LogPort, ExecPort, PortForwardPort, MetricsPort, PromqlPort, DescribePort, HelmPort, StatePort, SecretStorePort, NotifierPort, UpdaterPort, CrashReporterPort, IntegrationPort, AgentPort (ACP), ToolPort (MCP tool def+invoke), ContextProviderPort, FsPort, ClockPort; all async_trait, object-safe, with `oxikube_testkit` fakes
├── crates/app/oxikube_app                services: ClusterSessionManager, ResourceStore (reflector caches + watch budget + namespace scoping), CommandBus (dispatch + undo-able where sensible), MutationGuard (read-only, confirmation policy, dry-run-first, audit), LogService (single/aggregate/parse), ExecService, PortForwardManager, EventService, MetricsService, HelmService, DescribeService, SearchService (aliases, ':' parsing, fuzzy), IntegrationRegistry, ToolRegistry (MCP tools incl. per-integration), ContextRegistry (@-mentions), AgentSessionManager, SettingsFacade (reads via port), AuditService
├── crates/adapters/
│   ├── oxikube_kube          kubeconfig loader (kdash-derived), ClientPool per context w/ exec auth, discovery (aggregated), typed+dynamic APIs, reflectors (kube-runtime), Table API feed, logs, exec/attach/portforward (ws), SSA/dry-run/patch, subresources, metrics (k8s-metrics), events (core + events.k8s.io)
│   ├── oxikube_describe      deskribe adapter + kubectl-describe fallback
│   ├── oxikube_helm          release Secret/ConfigMap decoder (native) + helm CLI runner
│   ├── oxikube_prometheus    PromQL client, provider auto-detect (kube-prometheus, Lens stack, VictoriaMetrics, Mimir)
│   ├── oxikube_cloud         aws/gcloud/az CLI discovery adapters
│   ├── oxikube_argocd        CRD-direct (kopium types), REST client, core helper, SSO/token auth, Rollouts
│   ├── oxikube_state_sqlite  rusqlite StatePort + migrations + audit log
│   ├── oxikube_keychain      keyring SecretStorePort
│   ├── oxikube_notify_os     OS notifications
│   ├── oxikube_updater       GitHub Releases checker/installer
│   ├── oxikube_crash         sentry opt-in CrashReporterPort
│   ├── oxikube_acp           agent-client-protocol client, registry fetch, launchers (npx/uvx/binary+checksum)
│   └── oxikube_mcp           MCP server (rmcp) exposing ToolRegistry to agents (stdio/HTTP for ACP passthrough)
├── crates/platform/
│   ├── oxikube_runtime       gpui_tokio bridge, spawn_kube (abort-on-drop), channels, frame-coalesced notify
│   ├── oxikube_settings      SettingsStore (default → user → per-cluster), JSONC, comment-preserving edits, schemars schema, hot reload (vendored Zed logic)
│   ├── oxikube_keymap        keymap JSON, contexts, action registry, per-OS defaults, vim keymap
│   ├── oxikube_theme         Zed theme-family importer → tokens, ThemeRegistry, system appearance, k8s status colours
│   ├── oxikube_assets        embedded icons (Lucide), fonts
│   ├── oxikube_extension_api WIT + SDK for plugin authors (publishable)
│   ├── oxikube_extension_host wasmtime host, manifest, capabilities, install from path/git, epoch interruption
│   └── oxikube_logging       tracing, rolling file logs, redaction
├── crates/ui/
│   ├── oxikube_ui            thin wrapper over gpui-component: tokens, Table/Dock/Editor glue, icons, sizes-for-zoom
│   ├── oxikube_workspace     window shell, Item/Panel traits, panes, docks, tabs, status bar, modal/toast layers, layout persistence, cluster tabs
│   ├── oxikube_palette       command palette, ':' jump, pickers (vendored Zed PickerDelegate design)
│   ├── oxikube_catalog_ui    home/catalog, hotbar, kubeconfig sources mgmt, cloud discovery UI, connect lifecycle
│   ├── oxikube_resources_ui  generic resource table + detail drawer (structured/describe/YAML/events), per-kind panels & actions, create/bulk
│   ├── oxikube_overview_ui   cluster overview dashboard, charts, pulses
│   ├── oxikube_logs_ui       log viewer (single/aggregate/JSON), search, export, send-to-agent
│   ├── oxikube_terminal      alacritty_terminal element + TerminalBackend (local PTY, kube exec/attach)
│   ├── oxikube_editor        manifest editor (schema validation, diff, dry-run, apply, templates)
│   ├── oxikube_portforward_ui, oxikube_events_ui, oxikube_helm_ui, oxikube_rbac_ui
│   ├── oxikube_settings_ui   GUI settings page + keymap editor + theme picker
│   ├── oxikube_extensions_ui extensions manager
│   ├── oxikube_argocd_ui     Argo CD + Rollouts views
│   └── oxikube_agent_ui      ACP agent panel (threads, tool calls, permissions, diffs, terminals, @-mentions)
├── crates/testing/oxikube_testkit   port fakes, fixtures (JSON manifests), kind helpers, gpui test helpers
├── extensions/               sample extensions (theme, command, mcp-server)
├── docs/ ARCHITECTURE.md, CONTEXT.md (domain glossary), adr/0001…, research/ (the four research reports, saved during execution)
├── .claude/skills/oxikube-contributor/SKILL.md (+ references/), CLAUDE.md, AGENTS.md
└── .github/workflows/ ci.yml (fmt, clippy -D warnings, test, deny, lint-deps on ubuntu+macos), integration.yml (kind on ubuntu), nightly.yml (full matrix + screenshots), release.yml (cargo-dist)
```

## Epics (GitHub issues, type Epic) — phase order

Phase 0 — Foundation: E01 Workspace & tooling foundation · E02 Domain model, ports & testkit · E03 Kube adapter: connection, auth & discovery · E04 Kube adapter: resource data plane · E05 App shell (workspace, panes/docks/tabs, runtime bridge, settings core, keymap core, theme core)
Phase 1 — Core Kubernetes: E06 Cluster catalog & sessions · E07 Resource browser (generic table + detail) · E08 Logs · E09 Terminal & exec · E10 Manifest editor · E11 Command palette, ':' jump & keymaps
Phase 2 — Lens parity: E12 Per-kind panels & actions (workloads/config/network/storage/RBAC/CRDs) · E13 Metrics & cluster overview · E14 Events & notifications · E15 Port-forward manager · E16 Helm · E17 RBAC & access tooling · E18 Cloud discovery · E19 Safety & audit · E20 Apply/kustomize, file transfer & cross-cluster copy/diff
Phase 3 — Zed-style platform: E21 Settings & keymap UI · E22 Theming · E23 Extensions (WIT API, host, UI, samples) · E24 Release engineering & updates
Phase 4 — Integrations: E25 Integration framework + Argo CD + Rollouts
Phase 5 — Agents: E26 Agent foundation (MCP tool server, context providers, command exposure) · E27 ACP client & agent panel
Phase 6 — Later: E28 Windows support · E29 Backlog (Flux, extension registry, cross-cluster views, accessibility, image scanning)

Story drafts per epic: see "Epic & story catalogue" at the end of this file.

## Execution steps (after approval) — 29 epics, 364 stories

1. **Repo**: `gh repo create karan-vk/Oxikube --public --license gpl-3.0 --description "Native Kubernetes desktop client in Rust + GPUI"`. Init git in `/Users/karan-vijayakumar/code/0misc/Oxikube` (nested in the parent `code` repo, which already leaves `0misc/` untracked) and push `main`. Branch protection: require `ci` checks, squash-only, no force-push.
2. **Scaffold** (E01-S01…S08 "done in bootstrap"): workspace `Cargo.toml` (edition 2024, `[workspace.dependencies]` with the pins from the technical defaults, Zed-style lints/profiles), `rust-toolchain.toml` (1.99), `deny.toml`, `.cargo/config.toml`, every crate from the layout as an empty lib with README, `bins/oxikube` placeholder main, `xtask` with `lint-deps` (layer rules + gpui/kube/gpui_component bans) and `check-gpui-pin`, `.github/workflows/{ci,integration,nightly,release}.yml` (integration/nightly/release as stubs that E01/E24 complete), issue/PR templates, CODEOWNERS, `LICENSE-GPL`, `THIRD_PARTY_NOTICES.md`, `docs/ARCHITECTURE.md`, `docs/CONTEXT.md`, `docs/adr/0001–0012`, `docs/research/` (the four research reports + Zed dependency table saved from this session), `docs/ROADMAP.md` (epic list + phases). Gate: `cargo check --workspace`, `cargo xtask lint-deps`, `cargo xtask check-gpui-pin`, `cargo deny check` green locally. Dependencies are declared in `[workspace.dependencies]` but not consumed by the empty crates yet except `bins/oxikube`, which does depend on `gpui-pre` + `gpui-component` to prove the stack resolves (first build may take minutes; acceptable).
3. **Skill** (skill-creator process, light evaluation): `.claude/skills/oxikube-contributor/SKILL.md` (<500 lines, pushy description) + `references/{architecture-rules,story-workflow,pr-checklist,testing,gpui-gotchas,zed-vendoring,github-project}.md`; `CLAUDE.md` and `AGENTS.md` both point at it and carry the 10 non-negotiables (layer rules, no gpui/kube outside allowed layers, MutationGuard for every mutation, tool stub + Command per action, no secrets on disk, exact GPUI pins, no self-dropping Tasks, GPL header on vendored Zed code, story = branch = PR, conventional commits). One sanity run: a subagent is handed E02-S01 with the skill and must produce a branch name, file plan and test plan that obey the rules; adjust the skill from what it gets wrong.
4. **GitHub Project**: `gh project create --owner karan-vk --title "Oxikube"`; single-select fields Status (Backlog/Ready/In Progress/In Review/Done), Phase (0–6), Area (domain/ports/app/adapters/platform/ui/bins/tooling), Priority (P0–P3), Size (S/M/L), Team (text); views Board-by-Status, Roadmap-by-Phase, Table-by-Area; link repo.
5. **Issues**: labels `epic`, `story`, `phase:0..6`, `area:*`, `size:S|M|L`; milestones M0–M6 per phase; 29 epic issues (body = the epic block from this plan) then 364 story issues (body = title, acceptance criteria, crates/modules, depends, epic link) attached as native sub-issues via GraphQL `addSubIssue`; every issue added to the project with fields set. Done via a generated script in the scratchpad with retry/backoff (≈800 API calls; well under rate limits). Story IDs (E07-S03) stay in titles so dependencies are greppable.
6. **Memory**: save project memory (repo URL, project URL, plan file path, the decision table pointer) and a feedback memory that research agents run on Sonnet.

## Verification
- Local + CI: `cargo check --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo xtask lint-deps`, `cargo xtask check-gpui-pin`, `cargo deny check`, `cargo test --workspace` green on first push (ubuntu + macos).
- `gh issue list --label epic --limit 50` returns 29; per-epic sub-issue counts match the table above (364 total); `gh project item-list` shows fields populated; the three views render.
- Skill sanity run passes (step 3); `AGENTS.md`/`CLAUDE.md` reference resolves.
- Plan file archived into `docs/PLAN.md` in the repo so teams can read the full catalogue offline.

## Epic & story catalogue

### E01 — Workspace & tooling foundation  (Phase 0, Area: tooling)
**Goal:** Give every agent team a compiling, lint-enforced workspace where the hexagonal boundaries, GPUI pin discipline, CI gates and contributor rules exist before any feature code. Most of this is created during the bootstrap of this turn; the remaining stories add cluster test scaffolding and developer ergonomics.
**In scope:** Cargo workspace with every planned crate (empty), xtask (`lint-deps`, `check-gpui-pin`), CI/nightly/integration/release workflow files, cargo-deny, rust-toolchain, ADRs + ARCHITECTURE.md + CONTEXT.md, contributor skill + CLAUDE.md/AGENTS.md, kind scripts, load/churn script, screenshot harness, profiles, pre-commit, issue/PR templates.
**Out of scope:** cargo-dist packaging and auto-update (E24); settings schema generation xtask (E21); WIT binding generation xtask (E23).
**Crates:** xtask, all crates (skeleton), bins/oxikube. **Depends on:** none.
**Done when:** `cargo check --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo xtask lint-deps`, `cargo xtask check-gpui-pin`, `cargo deny check` are green on ubuntu + macos in CI; `cargo xtask kind-up && cargo test -p oxikube_kube --features integration` passes on ubuntu; nightly produces a screenshot artefact; the skill is loadable and referenced from CLAUDE.md/AGENTS.md.
**Risks:** GPUI dependency tree makes first CI builds slow → cache `target/` with `Swatinem/rust-cache`, set dev `opt-level=3` for proc-macro/tree-sitter/taffy/resvg/wasmtime packages (Zed pattern). Linux runner lacks Vulkan/Wayland dev packages → pin the apt list from Zed `script/linux` + Kubyl CI.

| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E01-S01 | Workspace skeleton with all crates (done in bootstrap) | M | • every crate in the layout exists with `lib.rs` + README stating responsibility and allowed deps<br>• `[workspace.dependencies]` pins gpui-pre =0.3.7, gpui-component =0.7.0, kube 4.2 (explicit features, `ring`), k8s-openapi 0.28 `latest`<br>• `[workspace.lints]` Zed-style (todo/dbg deny) applied via `[lints] workspace = true` in each crate | Cargo.toml, crates/*, bins/oxikube | – |
| E01-S02 | `cargo xtask lint-deps` dependency-direction lint (done in bootstrap) | M | • parses `cargo metadata`, encodes layer rules from ARCHITECTURE.md<br>• fails on domain→anything, ports→non-domain, app→gpui/kube/adapters, adapters→app/ui, ui→adapters<br>• unit-tested with a fixture metadata JSON | xtask/src/lint_deps.rs | S01 |
| E01-S03 | `cargo xtask check-gpui-pin` (done in bootstrap) | S | • verifies gpui-pre-* and gpui-component/base/assets pins are exact `=` and mutually aligned to the table in docs/adr/0004<br>• prints the Zed commit named by the snapshot | xtask/src/check_gpui_pin.rs | S01 |
| E01-S04 | CI workflows (done in bootstrap) | M | • ci.yml: fmt, clippy -D warnings, test, deny, lint-deps, check-gpui-pin on ubuntu-latest + macos-latest<br>• rust-cache; Linux apt deps installed<br>• required status checks documented for branch protection | .github/workflows/ci.yml | S01 |
| E01-S05 | cargo-deny + THIRD_PARTY_NOTICES (done in bootstrap) | S | • deny.toml allow-list: GPL-3.0-or-later, Apache-2.0, MIT, BSD-*, ISC, MPL-2.0, CC0-1.0, Unicode-3.0; advisories deny<br>• THIRD_PARTY_NOTICES.md with kdash MIT, deskribe NOTICE, Zed GPL vendoring section | deny.toml, THIRD_PARTY_NOTICES.md | S01 |
| E01-S06 | ADRs, ARCHITECTURE.md, CONTEXT.md (done in bootstrap) | M | • ADR 0001–0012 cover license, hexagonal layers, GPUI pin strategy, gpui-component wrapper, domain Resource model, Table hybrid, plugins WASM, ACP/MCP, Argo backends, persistence, metrics, safety<br>• CONTEXT.md glossary of domain terms (ClusterSession, Feed, Command, Capability…) | docs/ | – |
| E01-S07 | Contributor skill + CLAUDE.md + AGENTS.md (done in bootstrap) | M | • `.claude/skills/oxikube-contributor/SKILL.md` <500 lines with references/ for layer rules, story workflow, PR checklist, testing, GPUI gotchas, Zed-vendoring rules<br>• CLAUDE.md and AGENTS.md point to it; sanity run passes | .claude/skills, CLAUDE.md, AGENTS.md | S06 |
| E01-S08 | Repo hygiene templates (done in bootstrap) | S | • issue templates (story, bug), PR template with checklist mirroring the skill<br>• CODEOWNERS, branch protection documented (squash only, CI required) | .github/ | – |
| E01-S09 | `xtask kind-up/kind-down` + integration workflow | M | • creates a kind cluster with metrics-server and a sample CRD with printer columns, applies fixtures from testkit<br>• integration.yml runs `cargo test --features integration` on ubuntu per PR touching adapters/<br>• teardown always runs | xtask/src/kind.rs, .github/workflows/integration.yml | S04 |
| E01-S10 | `xtask load-pods --count N --churn` | S | • creates N pause pods across namespaces; `--churn` continuously deletes/recreates<br>• documented as the perf fixture for E07 perf story | xtask/src/load_pods.rs | S09 |
| E01-S11 | Screenshot harness | M | • `OXIKUBE_SCREENSHOT=out.png` with `--features screenshot` renders via `Window::render_to_image` (gpui + gpui_platform test-support)<br>• nightly.yml uploads screenshots as artefacts on macos + ubuntu (xvfb/wayland headless) | bins/oxikube, testkit, nightly.yml | S01 |
| E01-S12 | Developer ergonomics | S | • `release-fast` and `dbg` profiles; `.cargo/config.toml` aliases (`cargo x`, `cargo it`)<br>• pre-commit config (fmt, clippy, deny) + `xtask setup` installing it; optional `mise.toml` pinning rust | Cargo.toml, .cargo, .pre-commit-config.yaml | S01 |
| E01-S14 | Performance harness: `--perf` frame/feed/notify instrumentation, `xtask perf <scenario>`, baseline + nightly regression gate | M | • `oxikube --perf` writes per-frame times, feed throughput, notify counts to perf/*.jsonl and prints p50/p95/p99 on exit<br>• `cargo xtask perf scroll-10k\|palette\|startup\|logs-stream\|editor-5mb` runs scripted scenarios headless (TestApp + load-pods fixtures)<br>• docs/perf/baseline.json committed; nightly.yml fails on >20% regression; numbers linked from docs/PERFORMANCE.md | bins/oxikube, oxikube_runtime, oxikube_testkit, xtask, nightly.yml | S10, S11, E05-S11 |
| E01-S13 | Nightly full matrix | S | • nightly.yml: ubuntu + macos, debug + release, tests + screenshots + `cargo doc`<br>• failures open/append a tracking issue via gh | nightly.yml | S04, S11 |

### E02 — Domain model, ports & testkit  (Phase 0, Area: domain/ports)
**Goal:** Define the dependency-free core: identities, the thin Resource model, view-models, the command/capability vocabulary, every port trait, and a testkit of fakes so app and UI crates can be built and tested before real adapters exist. Signatures for later-phase ports (integration, tools, agent) are fixed now so no epic forces a domain change.
**In scope:** oxikube_domain types; oxikube_ports traits; oxikube_testkit fakes/fixtures; error taxonomy; property tests for parsers.
**Out of scope:** any I/O or kube-rs usage (E03/E04); gpui helpers in testkit (E05).
**Crates:** oxikube_domain, oxikube_ports, oxikube_testkit. **Depends on:** E01.
**Done when:** domain has zero internal deps and no I/O crates (lint-deps proves it); every port has a fake in testkit used by at least one test; Quantity parser passes the apimachinery test corpus; `cargo doc` builds without warnings; CONTEXT.md terms match type names.
**Risks:** Over-modelling 200+ kinds → only summaries for core kinds, everything else stays JSON. Port signatures churn when adapters land → ports reviewed against kube-rs/ACP/MCP APIs in this epic (S08–S10 cite the external APIs they wrap).

| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E02-S01 | Identity & kind types | S | • `ClusterId`, `ContextName`, `Gvk`, `Gvr`, `Scope{Cluster,Namespaced}`, `ResourceRef{cluster,gvk,ns,name}` with Display/Ord/Hash<br>• `ResourceKind` registry record (plural, singular, shortnames, categories, verbs, namespaced)<br>• serde round-trip tests | oxikube_domain::ids, ::kinds | – |
| E02-S02 | Resource model | M | • `Resource{meta: ObjectMeta(name, ns, uid, rv, labels, annotations, owner_refs, finalizers, creation, deletion), kind: Gvk, json: serde_json::Value}`<br>• JSON-pointer accessors, `strip_managed_fields()`, `to_yaml()` via serde-saphyr<br>• tests on Pod/Deployment/CR fixtures | oxikube_domain::resource | S01 |
| E02-S03 | Quantity & age | M | • `Quantity` parser per Kubernetes spec (binary/decimal SI, m/n/u, exponents) with arithmetic + percent; ported from kubectl-view-allocations `qty::Qty` (CC0) with attribution<br>• `Age` formatter on jiff ("5d3h"), tests from kdash `to_age` corpus<br>• proptest round-trips | oxikube_domain::quantity, ::age | – |
| E02-S04 | View-models for core kinds | L | • `PodSummary` (phase/status string per kubectl printer rules incl. Init:N/M, CrashLoopBackOff, Terminating, NodeLost; restarts; ready; qos; ip; node), `ContainerSummary`, `NodeSummary` (roles, conditions, schedulable, versions), `WorkloadSummary` (ready/desired/updated/available), `JobSummary`, `CronJobSummary`<br>• each built from `Resource` JSON, not k8s-openapi<br>• table-driven tests from fixtures; kdash `get_status` NOT copied (known init bug) | oxikube_domain::view | S02 |
| E02-S05 | Session & namespace state | S | • `ClusterSessionState{Disconnected,Connecting,AuthRequired,Ready,Degraded,Error}` state machine with allowed transitions<br>• `NamespaceSelection{All, Set(BTreeSet)}` + favourites; `WatchScope` derivation<br>• unit tests | oxikube_domain::session | S01 |
| E02-S06 | Command & capability vocabulary | M | • `CommandId`, `CommandMeta{title, scope, mutating, confirm: None/Simple/TypeName, needs: Capabilities}`<br>• `Capability` bitflags (mutate, exec, logs, portforward, helm, argo, …)<br>• `Command` payload enum covering navigation + resource verbs; designed so MCP tools and keymap actions map 1:1 | oxikube_domain::command | S01 |
| E02-S07 | Telemetry-free records | S | • `LogLine{ts, pod, container, text, parsed: Option<Json>}`, `Event` (merged core/events.k8s.io shape), `MetricsSample`, `AuditRecord{who, cluster, cmd, target, dry_run, outcome, ts}`, `ContextBlock{title, mime, body}`<br>• serde + size bounds | oxikube_domain::{log,event,metrics,audit,agent} | S01 |
| E02-S08 | Data-plane ports | M | • `ResourcePort` (list w/ pagination+selectors, get, create, replace, patch{Merge,Strategic,Json,Apply}, dry_run, delete{propagation}, delete_collection, subresource ops), `DiscoveryPort`, `TableFeedPort`, `WatchFeed` stream type, `LogPort`, `ExecPort` (stdin/stdout/resize), `PortForwardPort`<br>• async_trait, object-safe, `OxiError`<br>• signatures reviewed against kube-rs 4.2 `Api` methods | oxikube_ports::{resource,discovery,table,log,exec,portforward} | S01,S02,S12 |
| E02-S09 | Infrastructure ports | M | • `MetricsPort`, `PromqlPort`, `DescribePort`, `HelmPort`, `StatePort` (kv + typed tables + audit), `SecretStorePort`, `NotifierPort`, `UpdaterPort`, `CrashReporterPort`, `ClusterSourcePort`, `CloudDiscoveryPort`, `FsPort`, `ClockPort`<br>• doc comments state the adapter expected | oxikube_ports::* | S01,S07,S12 |
| E02-S10 | Integration/agent ports | M | • `IntegrationPort{id, detect(session)->Capabilities, sidebar(), commands(), tools(), context_providers(), settings_schema()}`<br>• `ToolPort{def: ToolDef(json schema), invoke}` shaped after MCP tool semantics; `ContextProviderPort{mention_prefix, resolve}`; `AgentPort` shaped after ACP client duties (session lifecycle, prompt, cancel, permission callback, fs/terminal callbacks)<br>• compile-time reviewed against agent-client-protocol 2.2 and rmcp types without depending on them | oxikube_ports::{integration,tool,context,agent} | S06,S07 |
| E02-S11 | Testkit fakes & fixtures | L | • `Fake*` for every port with scripted responses and recorded calls; `FakeResourcePort` replays watch event scripts with timing<br>• fixtures/: ≥30 JSON manifests (pods in each status, deployments, nodes, CRD+CR, events, helm release secret)<br>• builder helpers (`pod().running().restarts(3)`) | oxikube_testkit | S08–S10 |
| E02-S12 | Error taxonomy | S | • `OxiError{kind: Auth\|Forbidden\|NotFound\|Conflict\|Network\|Timeout\|Validation\|Unsupported\|Internal, message, source, retryable}`<br>• mapping guidelines in docs/ARCHITECTURE.md; `From` helpers reserved for adapters | oxikube_domain::error | – |
| E02-S13 | Domain docs & glossary sync | S | • rustdoc for every pub type; `#![deny(missing_docs)]` in domain/ports<br>• CONTEXT.md updated; `cargo doc` clean | oxikube_domain, docs/CONTEXT.md | S01–S12 |


### E03 — Kube adapter: connection, auth & discovery  (Phase 0, Area: adapters)
**Goal:** Implement the kube-rs side of connecting to many clusters at once: tolerant kubeconfig loading with hot reload, a per-context client pool with exec/OIDC auth, health probing, and aggregated API discovery feeding the domain kind registry. This is the first real adapter and proves the ports from E02 against a kind cluster.
**In scope:** `ClusterSourcePort` (kubeconfig), `ClientPool`, auth error classification, `DiscoveryPort`, connection health, TLS/proxy knobs, secrets hygiene, kind integration tests.
**Out of scope:** cloud CLI discovery (E18); resource CRUD/watch (E04); UI for sources/connect (E06).
**Crates:** oxikube_kube (modules `kubeconfig`, `pool`, `auth`, `discovery`, `health`), oxikube_logging (redaction hook). **Depends on:** E01, E02.
**Done when:** two kind contexts in one KUBECONFIG connect concurrently; editing the kubeconfig file updates the context list within 2s; discovery lists every served kind incl. a CRD and re-runs when a CRD is added; an RBAC-restricted service account yields `Forbidden` errors not panics; no token appears in logs (redaction test).
**Risks:** Interactive exec plugins (MFA prompts) block → expose `ExecInteractiveMode` policy to the session and surface `AuthRequired` with the plugin's message. Aggregated discovery unsupported on old servers → fallback to `Discovery::run()`.

| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E03-S01 | Tolerant kubeconfig loader | S | • port of kdash `load_kubeconfig_from_paths`/`is_blank_kubeconfig` (MIT header kept): splits KUBECONFIG, skips blank/missing files, `Kubeconfig::merge`<br>• unit tests: blank file, missing path, duplicate context names, relative cert paths | oxikube_kube::kubeconfig | – |
| E03-S02 | ClusterSourcePort: kubeconfig sources + hot reload | M | • sources: default path, $KUBECONFIG, user-added files/dirs (from settings), pasted kubeconfigs stored under config dir<br>• `notify` watcher + 60s safety poll (kdash pattern) emits `SourcesChanged` with diff<br>• deterministic tests use a manual `reload()` without watcher | oxikube_kube::sources | S01 |
| E03-S03 | ClientPool per context | M | • `Config::from_custom_kubeconfig(KubeConfigOptions{context})`, proxy_url/HTTPS_PROXY fallback, connect/read timeouts, `RetryPolicy::server_retry`, gzip<br>• lazy build, `Arc<Client>` reuse, invalidate on source change, LRU eviction of idle clients<br>• unit tests with fake kubeconfig | oxikube_kube::pool | S01 |
| E03-S04 | Auth handling & error classification | M | • exec plugins (kube `ExecConfig`), OIDC/oauth features enabled; `ExecInteractiveMode` policy param<br>• classify 401/exec failures → `OxiError::Auth{retryable}`; 403 → Forbidden (adapt kdash `should_retry_kubectl_refresh` idea without shelling to kubectl)<br>• tests with recorded error bodies | oxikube_kube::auth | S03 |
| E03-S05 | Connection health & capabilities probe | M | • `apiserver_version`, `SelfSubjectRulesReview` per namespace (cached), optional `SelfSubjectAccessReview` helper<br>• periodic liveness with backoff driving `ClusterSessionState` transitions (Ready/Degraded)<br>• kind test: stop API access (bad token) → Degraded then Error | oxikube_kube::health | S03,S04 |
| E03-S06 | DiscoveryPort | M | • `Discovery::new(client).run_aggregated()` with fallback to `run()`; builds `ResourceKind` registry (preferred version, verbs, scope, shortnames, categories)<br>• `resolve(Gvk)->ApiResource` cache; CRD watcher triggers incremental re-discovery<br>• kind test with sample CRD | oxikube_kube::discovery | S03 |
| E03-S07 | TLS & proxy edge cases | S | • insecure-skip-tls-verify, custom CA data/file, `proxy-url` in kubeconfig, socks5, `tls-server-name`<br>• integration test via kind with a mitm proxy container or unit-level config assertions | oxikube_kube::pool | S03 |
| E03-S08 | Secrets hygiene & redaction | S | • tracing layer redacts bearer tokens, client-key-data, Authorization headers<br>• `Debug` impls for configs never print secrets; test asserts log output | oxikube_logging, oxikube_kube | S03 |
| E03-S09 | kind integration suite: connect/discover/RBAC | M | • two contexts, concurrent connect; discovery snapshot; restricted SA; hot reload by rewriting kubeconfig<br>• runs under `--features integration` in integration.yml | oxikube_kube/tests | S02–S06 |
| E03-S10 | In-cluster & env edge inputs | S | • `Config::incluster()` fallback when no kubeconfig (for future headless use), `KUBECONFIG` with `:`/`;` separators on all OS<br>• tests | oxikube_kube::kubeconfig | S01 |

### E04 — Kube adapter: resource data plane  (Phase 0, Area: adapters)
**Goal:** Implement every data-plane port on kube-rs: paginated list/get, reflector-backed watch feeds with budgets, the hand-rolled Table API feed for CRD printer columns, all mutation forms (SSA, dry-run, patches, delete), subresources and kubectl-equivalent algorithms (trigger cronjob, rollout undo, drain), logs with reconnect, exec/attach/port-forward over websockets, metrics and merged events.
**In scope:** `ResourcePort`, `TableFeedPort`, `LogPort`, `ExecPort`, `PortForwardPort`, `MetricsPort`, events feed, watch budget, kind integration suite.
**Out of scope:** app-level caching/filters (E07), log UI (E08), terminal UI (E09), Prometheus (E13), describe (E07 via oxikube_describe).
**Crates:** oxikube_kube (modules `resources`, `feed`, `table`, `mutate`, `subresource`, `algorithms`, `logs`, `remote`, `metrics`, `events`, `budget`). **Depends on:** E02, E03.
**Done when:** every port method has a kind integration test; a CRD with additionalPrinterColumns renders the server's columns through `TableFeedPort`; SSA apply with conflict returns `Conflict` with field managers; dry-run returns the server diff object; logs survive a pod restart with no duplicated lines; exec round-trips stdin/stdout with resize; metrics absence is reported as `Unsupported` not swallowed.
**Risks:** Table watch semantics differ from typed watchers → feed abstraction hides refresh strategy (sofka: list+watch with 30s refresh); gate StreamingList on server feature. Memory on large clusters → watch budget + metadata-only feeds + load-pods test.

| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E04-S01 | List/get with pagination & selectors | M | • typed (`Api<K>`) and dynamic (`Api<DynamicObject>` via `ApiResource`) paths behind one impl; `ListParams` limit/continue, label/field selectors, `resourceVersion` semantics<br>• converts to domain `Resource` (strip managedFields optional)<br>• kind test lists 2k pods in pages | oxikube_kube::resources | – |
| E04-S02 | Reflector watch feed | L | • `kube_runtime::watcher` + `reflector` Store per (cluster, gvk, scope); `watcher::Config` page_size, `streaming_lists()` when server ≥1.32 feature detected, backon backoff<br>• emits coalesced `WatchEvent{Applied,Deleted,Restarted}` batches on a bounded channel<br>• unit test with fake API via kube's `tower::mock`; kind test with churn | oxikube_kube::feed | S01 |
| E04-S03 | Metadata-only feed | S | • `metadata_watcher`/`Api<PartialObjectMeta>` variant for cheap lists; upgrade-to-full on demand<br>• tests | oxikube_kube::feed | S02 |
| E04-S04 | Table API feed | L | • builds `kube::core::Request` list/watch with Accept `application/json;as=Table;v=v1;g=meta.k8s.io,application/json` + `includeObject=Metadata` (sofka/kubetui prior art); own `Table/TableRow/ColumnDefinition` types<br>• list+watch loop with refresh interval + fallback to JSON when Accept ignored<br>• kind test on sample CRD printer columns and on pods (`-o wide` parity) | oxikube_kube::table | S01 |
| E04-S05 | Mutations | M | • create, replace (rv check), patch Merge/Strategic/Json, `Patch::Apply` with field manager `oxikube` + force flag, server dry-run returning the resulting object, delete with propagation policy, delete_collection<br>• error mapping (409→Conflict with managers; 422→Validation with field paths)<br>• kind tests incl. SSA conflict | oxikube_kube::mutate | S01 |
| E04-S06 | Subresources & patch builders | M | • scale get/patch, status, evict (`Eviction`), ephemeral containers patch, pod resize<br>• patch builders ported from kdash `ResourcePatch::to_merge_patch` (rollout restart annotation, cordon/uncordon, cronjob suspend) with tests | oxikube_kube::subresource | S05 |
| E04-S07 | kubectl-equivalent algorithms | M | • `trigger_cronjob` (kdash port: jobTemplate → Job with generateName + ownerRef), `rollout_undo` (find RS by `deployment.kubernetes.io/revision`, patch template), `rollout_history`, `drain` (cordon → evict with PDB retry/backoff, ignore daemonsets, delete emptyDir flag)<br>• kind tests for each | oxikube_kube::algorithms | S05,S06 |
| E04-S08 | LogPort | M | • `Api<Pod>::log_stream` with `LogParams` follow/since/tail/previous/timestamps; reconnect with overlap + dedup (kdash `stream.rs:126-307` logic, MIT header) as a `Stream<LogLine>`<br>• multi-container and label-selector fan-in helpers<br>• kind test: restart pod mid-stream → no dupes, no gap | oxikube_kube::logs | S01 |
| E04-S09 | ExecPort & attach | M | • `Api<Pod>::exec/attach` with `AttachParams` (tty, stdin), `AttachedProcess` streams, `TerminalSize` channel, exit status<br>• node shell helper: privileged pod spec (configurable image) + exec + cleanup; debug container via ephemeral containers<br>• kind test echo round-trip + resize | oxikube_kube::remote | S06 |
| E04-S10 | PortForwardPort | M | • `Portforwarder` per pod port; service→pod resolution; local TCP listener bridging `take_stream`; error channel; restart hook when pod gone<br>• kind test: forward to nginx and GET | oxikube_kube::remote | S01 |
| E04-S11 | MetricsPort via k8s-metrics | S | • `Api<NodeMetrics>/<PodMetrics>` list; detect absence (404/503) → `Unsupported`; utilisation with domain `Quantity`<br>• thin internal fallback types if crate lags k8s-openapi<br>• kind test with metrics-server installed | oxikube_kube::metrics | S01 |
| E04-S12 | Events feed | S | • watch core/v1 Events and events.k8s.io/v1 merged into domain `Event`; per-object filter by involvedObject UID<br>• ring-buffer sized feed | oxikube_kube::events | S02 |
| E04-S13 | Watch budget & observability | M | • per-cluster limits (max feeds, max objects), idle feed teardown, namespace-scoped feeds when selection is a Set<br>• tracing spans + counters (feeds, objects, bytes); exported for E07 perf work<br>• unit tests with fake feeds | oxikube_kube::budget | S02–S04 |
| E04-S14 | kind integration suite: data plane | M | • covers S01–S12 end-to-end incl. Table on CRD, SSA conflict, dry-run, logs restart, exec resize, port-forward, eviction/drain with PDB<br>• wired into integration.yml | oxikube_kube/tests | S01–S13 |


### E05 — App shell  (Phase 0, Area: platform/ui)
**Goal:** Stand up the Zed-like shell every feature plugs into: the tokio↔GPUI runtime bridge, the `oxikube_ui` wrapper over gpui-component, window + workspace (Item/Panel/Pane/Dock with persistence), and the core of settings, keymap and theme systems (vendored Zed designs). After this epic, a feature crate adds itself with one `init(cx)` call and gets tabs, docks, settings, keybindings and theming for free.
**In scope:** oxikube_runtime, oxikube_ui, oxikube_workspace, oxikube_settings core, oxikube_keymap core, oxikube_theme core, oxikube_state_sqlite layout persistence, bins/oxikube init order, status bar/modal/toast layers, gpui test harness.
**Out of scope:** settings GUI/keymap editor/theme picker (E21/E22); command palette and ':' jump (E11); cluster tabs semantics (E06).
**Crates:** oxikube_runtime, oxikube_ui, oxikube_workspace, oxikube_settings, oxikube_keymap, oxikube_theme, oxikube_assets, oxikube_state_sqlite, oxikube_logging, bins/oxikube, oxikube_testkit. **Depends on:** E01, E02.
**Done when:** `cargo run` opens a themed window with left/right/bottom docks, a centre pane group, status bar; layout survives restart via SQLite; `settings.json` edits hot-reload within 1s; `keymap.json` rebinding works; a Zed theme file dropped in `themes/` appears and applies; screenshot test green on macOS + Linux CI.
**Risks:** gpui-component Root/overlay and absolute-pixel resizables (Kubyl gotchas) → `oxikube_ui` owns Root rendering and a `u(px)` size helper. GPUI test non-determinism from OS threads → runtime exposes a test mode with no watchers.

| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E05-S01 | Runtime bridge | M | • `gpui_tokio::init`, `spawn_kube(cx, fut)` returning a GPUI Task that aborts the tokio task on drop<br>• `notify_coalesced` helper batching `cx.notify()` to frame cadence<br>• test proving a Task that clears its own slot does not self-cancel (flag + detach pattern documented) | oxikube_runtime | – |
| E05-S02 | `oxikube_ui` wrapper | M | • re-exports a curated API (Table, DockArea, Dialog, Menu, Input, Tabs, Sidebar, Charts, Markdown) so feature crates never import `gpui_component` (lint-deps ban)<br>• `Tokens` (colours, spacing, radius) + `u(px)` zoom-safe sizes; Lucide icon enum; `init(cx)` wiring gpui-component theme from `oxikube_theme` | oxikube_ui, oxikube_assets | – |
| E05-S03 | Window & Root | M | • titlebar (custom on Linux with client-side decorations; native on macOS), gpui-component Root layers rendered once, macOS app menu via `cx.set_menus`<br>• Wayland app_id + .desktop asset; HiDPI check<br>• screenshot test | oxikube_workspace::window, bins/oxikube | S02 |
| E05-S04 | Workspace model: Item/Panel/Pane/Dock | L | • vendored Zed design: `Item` (tab content: title, icon, dirty, close), `Panel` (dock position, default size, toggle action), `Pane`/`PaneGroup` splits on gpui-component DockArea; drag-drop tabs; zoom; close/reopen<br>• `Workspace` entity API: `open_item`, `toggle_panel`, `active_pane`<br>• gpui tests for split/move/close | oxikube_workspace | S02,S03 |
| E05-S05 | Layout persistence | M | • `oxikube_state_sqlite`: rusqlite bundled, migrations table, `StatePort` impl (kv + typed rows)<br>• workspace serialises gpui-component `PanelState` + open items; restore on launch; corrupt-state fallback<br>• tests with temp db | oxikube_state_sqlite, oxikube_workspace::persistence | S04 |
| E05-S06 | Settings core | L | • `SettingsStore` global (vendored Zed logic, GPL header): layers default.json (embedded) → user settings.json (JSONC) → per-cluster overrides; `Settings` trait + inventory registration; `get::<T>()`, `observe`<br>• comment-preserving `update_user_settings` text edits; schemars `settings.schema.json` emitted by `xtask gen-settings-schema`<br>• notify hot reload (disabled in tests); unit tests for merge precedence | oxikube_settings, xtask | – |
| E05-S07 | Keymap core | M | • `keymap.json` format (Zed sections: context, bindings, null to unbind); per-OS defaults embedded; `cx.bind_keys` from merged layers; action registry with namespaces; `key_context` helpers for views<br>• vim.json optional layer flag<br>• tests: user override wins, unbind works | oxikube_keymap | S06 |
| E05-S08 | Theme core | M | • Zed theme-family JSON (schema v0.2.0) importer → `ThemeTokens` (map Zed `style` keys to our tokens; `oxikube` block for k8s status colours with defaults)<br>• `ThemeRegistry` (bundled One Dark/One Light + user `themes/` dir hot-reload), system appearance follow, `theme` setting {name \| mode/light/dark}<br>• maps tokens onto gpui-component `ThemeConfig`; tests import Ayu/Gruvbox from Zed assets | oxikube_theme | S06 |
| E05-S09 | AppState & init order | S | • `AppState` global (ports bundle, settings, theme, keymap, state db, runtime); `bins/oxikube` calls each crate's `init(cx)` in documented order (Zed main.rs pattern)<br>• panic hook → crash log file; `oxikube_logging` init with rolling files + redaction | bins/oxikube, oxikube_logging | S01,S05–S08 |
| E05-S10 | Status bar, modal & toast layers | S | • status bar with left/right item registry; modal layer (dialogs via oxikube_ui), toast/notification layer with queue<br>• focus model: focus handles + tab stops; escape closes modal<br>• gpui tests | oxikube_workspace::{status_bar,modal,toast} | S04 |
| E05-S11 | gpui test harness | M | • testkit: `TestApp` helpers (open workspace, simulate keystrokes, dispatch action, screenshot), fake AppState with testkit ports<br>• doc page "writing deterministic GPUI tests" (no OS threads, no self-dropping tasks)<br>• CI runs them on both OS | oxikube_testkit, docs | S09 |
| E05-S13 | Startup budget: ≤400 ms to first interactive frame, lazy init | M | • cold-start trace (perf harness) shows catalog window interactive ≤400 ms on M-series with 20 contexts; no network before first frame<br>• settings/keymap/theme load on the main thread stays <30 ms; SQLite open + layout restore async with placeholder<br>• crates' `init(cx)` order documented with measured cost; heavy inits (extension host, discovery) deferred until needed | bins/oxikube, oxikube_runtime, oxikube_workspace, oxikube_settings | S09, E01-S14 |
| E05-S12 | Window/session basics | S | • multiple windows allowed; `cmd/ctrl +/-` zoom via rem size; reduce-motion respect; quit confirm when operations running | oxikube_workspace | S04 |

### E06 — Cluster catalog & sessions  (Phase 1, Area: app/ui)
**Goal:** Make clusters first-class: a catalog home listing every kubeconfig context, a hotbar and one workspace tab per connected cluster, a connect lifecycle with auth prompts, Lens-style namespace selection remembered per cluster, per-cluster settings including read-only mode and colour, and a sidebar gated by what the user can actually list.
**In scope:** `ClusterSessionManager` (app), catalog UI, hotbar/cluster tabs, kubeconfig sources UI, connect lifecycle UI, namespace selector, per-cluster settings, read-only enforcement, sidebar skeleton, session restore.
**Out of scope:** cloud discovery (E18); resource views inside the sidebar (E07+); cluster overview page (E13).
**Crates:** oxikube_app (session, guard), oxikube_catalog_ui, oxikube_workspace (cluster tabs), oxikube_settings (per-cluster layer), oxikube_state_sqlite. **Depends on:** E03, E04, E05.
**Done when:** two clusters are connected side by side in separate tabs; a bad token shows AuthRequired with retry instead of a blank screen; namespace selection persists across restarts; read-only cluster hides/blocks every mutating command (tested through CommandBus); sidebar hides sections forbidden by RBAC.
**Risks:** Exec-plugin prompts (MFA) → policy setting per cluster + visible instructions. Many connected clusters → sessions lazy; idle feeds torn down (E04-S13).

| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E06-S01 | ClusterSessionManager | L | • app service owning `ClusterSession{id, ports bundle, state, caps, namespace selection, read_only, colour}`; connect/disconnect/reconnect; state machine from E02-S05; events stream<br>• unit tests with testkit fakes incl. AuthRequired path | oxikube_app::session | – |
| E06-S02 | MutationGuard & CommandBus core | M | • `CommandBus::dispatch(cmd, ctx)` routes to handlers; `MutationGuard` blocks mutating commands on read-only sessions, applies confirm policy, records `AuditRecord` via StatePort (full pipeline hardened in E19)<br>• handlers registry per crate `init`<br>• tests: read-only denial, confirm level by CommandMeta | oxikube_app::{command_bus,guard,audit} | S01 |
| E06-S03 | Catalog home view | M | • lists contexts from `ClusterSourcePort` with name/cluster/user/source file, status badge, last-used; search (nucleo); favourites; connect on enter/click<br>• empty state explains how to add kubeconfigs<br>• gpui tests with fake source | oxikube_catalog_ui::catalog | S01 |
| E06-S04 | Hotbar & cluster tabs | M | • left hotbar with connected/favourite clusters (colour dot, initials/icon); one workspace tab per connected cluster hosting its own sidebar + pane group<br>• switch via keymap (`cmd-1..9`), close tab = disconnect prompt<br>• layout persisted | oxikube_workspace::cluster_tab, oxikube_catalog_ui::hotbar | S03, E05-S04 |
| E06-S05 | Kubeconfig sources management | M | • settings-backed list of files/dirs; add via file picker; paste kubeconfig → stored at `<config>/kubeconfigs/<name>.yaml`; remove; reload<br>• invalid file shows error inline<br>• tests via fake Fs/ClusterSource | oxikube_catalog_ui::sources, oxikube_settings | S03 |
| E06-S06 | Connect lifecycle UI | M | • states: Connecting (spinner + server), AuthRequired (plugin message, open terminal to re-login, retry), Degraded banner, Error with details + retry<br>• interactive exec policy per cluster surfaced | oxikube_catalog_ui::connect | S01,S04 |
| E06-S07 | Namespace selector | M | • dropdown with All / multi-select / favourites / search; remembered per cluster via StatePort; '0-9' favourites keys (k9s)<br>• changes `WatchScope` → ResourceStore re-scopes feeds<br>• tests | oxikube_catalog_ui::namespaces, oxikube_app::session | S01 |
| E06-S08 | Per-cluster settings layer | M | • `clusters.<id>` override layer in SettingsStore: display name, colour, read_only, default namespace, terminal cwd, node shell image/pull secret, prometheus override, accessible namespaces<br>• schema + hot reload; tests | oxikube_settings, oxikube_app | S01 |
| E06-S09 | Read-only mode & colour badges | S | • toggle in tab context menu + settings; badge on tab/hotbar/status bar; prod presets (red)<br>• enforcement tests through CommandBus incl. future MCP mutation tools | oxikube_app::guard, oxikube_workspace | S02,S08 |
| E06-S10 | Cluster sidebar skeleton | M | • sections Cluster/Nodes/Workloads/Config/Network/Storage/Namespaces/Events/Helm/Access Control/Custom Resources with collapsible groups and counts placeholder<br>• visibility gated by `SelfSubjectRulesReview` results; integration sections appended by IntegrationRegistry (stub) | oxikube_workspace::sidebar | S04 |
| E06-S11 | Session restore | S | • on launch reopen previously connected clusters (opt-in setting), their tabs, namespaces; failures don't block startup<br>• tests | oxikube_app::session, oxikube_workspace::persistence | S01,S04 |
| E06-S12 | kind smoke test | S | • integration test: connect two kind contexts through the real adapter + app session (no UI), namespace scoping switches feeds | oxikube_app/tests | S01,S07 |


### E07 — Resource browser: generic table + detail  (Phase 1, Area: ui/app)
**Goal:** One generic, virtualised resource table and one generic detail drawer that work for every kind the cluster serves, including CRDs through server printer columns, with k9s-grade filtering and Lens-grade column management. Per-kind specialisations come later (E12); this epic makes "browse anything, read anything, delete anything" solid and fast at 10k objects.
**In scope:** `ResourceStore` app façade, `ColumnProvider` + core column defs, generic table view, filter bar, detail drawer with YAML/Describe/Events tabs, CRD browsing, row-action framework + delete, performance harness, states.
**Out of scope:** per-kind panels/actions beyond delete (E12); editor (E10); logs (E08); events page (E14); metrics columns values (E13 fills the hooks).
**Crates:** oxikube_app (store, columns), oxikube_resources_ui, oxikube_describe, oxikube_ui. **Depends on:** E04, E05, E06.
**Done when:** every kind from discovery opens as a table; a CRD shows its additionalPrinterColumns; `/` filter, `-l` selectors and sort work; column visibility persists; detail drawer shows structured metadata, YAML, describe (deskribe, kubectl fallback) and events; 10k pods with churn scroll at ≥55fps on the M5 Max with frame-time log; delete with confirmation and read-only block tested.
**Risks:** gpui-component Table churn → all usage inside `oxikube_ui::Table` glue + our `TableDelegate` impl. Table API refresh vs watch → ResourceStore hides the feed type.

| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E07-S01 | ResourceStore façade | L | • per-session cache keyed by (gvk, scope) choosing reflector/metadata/Table feed by kind policy; `subscribe(query)->Stream<Delta>`; in-app sort/filter/index by name/ns/labels<br>• ref-counted feeds auto-stop (budget hooks)<br>• tests with FakeResourcePort scripts | oxikube_app::store | – |
| E07-S02 | ColumnProvider & core columns | L | • `ColumnProvider{columns(kind, caps), cell(resource)->Cell}`; `CoreColumns` for ~40 kinds from the Freelens/k9s catalog (Pods: Name, Ns, Ready, Status, Restarts, Node, IP, QoS, CPU/Mem hooks, Age…)<br>• `TableColumns` provider for Table feed rows<br>• priority/wide flag; tests per kind from fixtures | oxikube_app::columns | S01 |
| E07-S03 | Generic ResourceTable view | L | • gpui-component Table (uniform rows) through oxikube_ui; sort by any column, resize/reorder/show-hide persisted per kind via StatePort; multi-select; context menu; keyboard nav (j/k, enter opens detail)<br>• status cell colours from theme `oxikube` block<br>• gpui tests | oxikube_resources_ui::table | S01,S02 |
| E07-S04 | Filter bar | M | • `/text`, `/!text` inverse, `/-l k=v` label selector (server-side), `/-f fuzzy`; namespace scoping from E06-S07; filter persisted per view (setting)<br>• debounced; tests on semantics | oxikube_resources_ui::filter, oxikube_app::store | S03 |
| E07-S05 | Detail drawer (generic) | M | • right drawer/tab with header (kind, name, ns, age, status), metadata (labels/annotations with copy, owner refs clickable, finalizers), conditions table, `status` summary from JSON<br>• tabs: Overview / YAML / Describe / Events; pin as tab item<br>• gpui tests | oxikube_resources_ui::detail | S03 |
| E07-S06 | YAML & Describe tabs | M | • YAML: read-only highlighted (gpui-component editor read-only, tree-sitter yaml), managedFields toggle, secrets masked, copy/save<br>• Describe: `DescribePort` via `oxikube_describe` (deskribe `fetch`), fallback `kubectl describe` adapter when configured/available; refresh<br>• tests with fakes | oxikube_resources_ui::detail, oxikube_describe | S05 |
| E07-S07 | CRD browsing | M | • Custom Resources sidebar grouped by API group with counts; CRD list view (group/version/scope/short names) → CR table via Table feed<br>• generic detail works on CRs; schema shown in CRD detail<br>• kind test with sample CRD | oxikube_resources_ui::crds, oxikube_workspace::sidebar | S03,S05 |
| E07-S08 | Row actions framework + delete | M | • actions resolved from CommandBus registry by kind + capabilities; context menu + palette exposure; disabled/hidden by read-only<br>• delete: propagation choice, 'type the name' for Namespace/Node/PV, bulk delete with per-item results<br>• tests through fake bus | oxikube_resources_ui::actions, oxikube_app::command_bus | S03, E06-S02 |
| E07-S09 | Performance harness & tuning | M | • `--perf` flag logs frame times + feed throughput; `xtask load-pods --count 10000 --churn` scenario documented<br>• coalesced deltas, incremental sort; target ≥55fps scrolling, <1s initial render after feed warm<br>• perf numbers recorded in docs | oxikube_resources_ui, oxikube_app::store | S03 |
| E07-S10 | States & diagnostics | S | • loading vs empty vs forbidden vs error states (k9s #4121 lesson), API-server `Warning:` headers surfaced as toast once<br>• retry button; tests | oxikube_resources_ui | S03 |
| E07-S11 | Workloads overview placeholder & sidebar counts | S | • sidebar counts from store; Workloads overview tiles (counts/healthy) using store only (metrics later)<br>• tests | oxikube_resources_ui::overview_lite | S01 |
| E07-S12 | gpui + store test suite | M | • table with scripted feed (add/modify/delete), filter semantics, selection, column persistence, detail tabs; screenshot of pods table | oxikube_resources_ui/tests | S03–S08 |

### E08 — Logs  (Phase 1, Area: app/ui)
**Goal:** A log viewer better than Lens and k9s: single-container and stern-style multi-pod streams with reconnect on churn, regex search, JSON structured mode, export, and from day one a context provider + MCP tool so hosted agents can read the same logs.
**In scope:** `LogService`, log viewer item, search, aggregation, JSON mode, export, reconnect-on-churn, kubectl fallback, agent context/tool registration, tests.
**Out of scope:** terminal (E09; S08 only opens a terminal tab), agent panel UI (E27), Prometheus/Loki backends (backlog E29).
**Crates:** oxikube_app (logs, context, tools), oxikube_logs_ui, oxikube_kube (LogPort from E04). **Depends on:** E04, E05, E07.
**Done when:** opening logs on a Deployment streams all its pods merged by timestamp with per-pod colours; restarting the deployment keeps the view following new pods with no duplicates; `/error|warn` highlights matches with next/prev; JSON lines render as level/time/message columns with a level filter; export writes the visible buffer; `@logs` context provider and `get_logs` tool are registered (verified via ToolRegistry test).
**Risks:** Memory on chatty pods → bounded ring buffer (setting `logs.buffer_lines`) with "truncated" marker. Clock skew across pods → merge by server timestamp with stable tiebreak.

| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E08-S01 | LogService | M | • sessions over `LogPort` keyed by target (pod/container or selector); options follow/since/tail/previous/timestamps; bounded ring buffer; line indexing<br>• emits batched deltas; cancel on drop<br>• unit tests with FakeLogPort | oxikube_app::logs | – |
| E08-S02 | Log viewer item | L | • workspace Item with virtualised lines (uniform_list), wrap toggle, autoscroll with "new lines" pill, timestamps toggle, container selector incl. init/ephemeral, previous-container toggle, since presets (1m/5m/15m/30m/1h), tail/head<br>• keymap context `LogView` (k9s keys 0-6, s, w, t, f, m, c)<br>• gpui tests | oxikube_logs_ui::view | S01 |
| E08-S03 | Search & filter | M | • regex + case toggle, highlight, next/prev, match count; inverse filter; filter persisted per session<br>• incremental over ring buffer; tests | oxikube_logs_ui::search, oxikube_app::logs | S02 |
| E08-S04 | Multi-pod aggregation | M | • open logs on workload/selector/service → fan-in via LogPort helpers; merge by timestamp with stable order; per-pod colour (hash → palette) and short-name prefix; pod add/remove banner<br>• tests on ordering with fixtures | oxikube_app::logs, oxikube_logs_ui | S01,S02 |
| E08-S05 | JSON structured mode | M | • detect JSON lines (per line), columns level/time/message + expandable fields, level filter chips, pretty-print on expand; mixed streams supported<br>• parser corpus tests (zap, logrus, bunyan, pino) | oxikube_app::logs::parse, oxikube_logs_ui | S02 |
| E08-S06 | Export, copy, mark, clear | S | • save visible or all to file (FsPort), copy selection, mark line (k9s `m`), clear; download respects filters<br>• tests | oxikube_logs_ui | S02 |
| E08-S07 | Reconnect & churn following | M | • when a followed pod terminates/replaces (Lens #8163 class), workload-targeted sessions pick up new pods; single-pod session shows ended state with "follow replacement" action<br>• kind test with rollout restart | oxikube_app::logs | S04 |
| E08-S08 | kubectl fallback in terminal | S | • action "Tail in terminal" opens a terminal tab running `kubectl logs -f …` with cluster env (needs E09)<br>• hidden if kubectl missing | oxikube_logs_ui, oxikube_terminal | E09-S03 |
| E08-S09 | Agent hooks: context provider + tool | M | • `LogContextProvider` (`@logs pod/ns/name [--since]`, selection → ContextBlock) registered in ContextRegistry; "Send to agent" action on selection (queues until E27)<br>• `get_logs` ToolPort def (pod/selector, since, tail, grep) in ToolRegistry; read-only safe<br>• tests through fakes | oxikube_app::{context,tools}, oxikube_logs_ui | S03,S04 |
| E08-S10 | Settings & keymap | S | • settings: buffer lines, default tail, wrap, timestamps, JSON auto-detect; keymap defaults + rebinding; schema updated | oxikube_logs_ui, oxikube_settings | S02 |
| E08-S11 | Test suite | S | • LogService unit (buffer bounds, reconnect overlap dedupe), aggregation ordering, parser corpus, gpui view tests, screenshot | oxikube_logs_ui/tests, oxikube_app/tests | S01–S07 |


### E09 — Terminal & exec  (Phase 1, Area: ui)
**Goal:** One terminal implementation (alacritty_terminal grid painted by our own GPUI Element) that serves local shells, pod exec/attach, node shells and ephemeral debug containers as movable workspace tabs. Every cluster tab's terminal starts with KUBECONFIG/context/namespace pre-set so kubectl/helm/argocd "just work".
**In scope:**
- `oxikube_terminal` crate: TerminalElement (custom `Element` painting cells, cursor, selection, scrollback, hyperlinks), keystroke→escape mapping, IME, resize, copy/paste, search, font/theme from settings.
- `TerminalBackend` trait with two impls: `LocalPty` (portable-pty) and `KubeStream` (kube `AttachedProcess` stdin/stdout/stderr + `TerminalSize` channel).
- ExecService in `oxikube_app` + `ExecPort` impl in `oxikube_kube` (exec, attach, node-shell via privileged pod, ephemeral debug container via `Api::<Pod>::patch_ephemeral_containers`).
- Terminal as a workspace `Item` (tab) draggable between panes and the bottom dock; multiple terminals; restore on launch (command + cwd only, never scrollback).
**Out of scope:** logs viewer (E08); agent terminal passthrough (E27); port-forward (E15); terminal settings GUI (E21).
**Crates:** oxikube_terminal, oxikube_app (exec service), oxikube_ports (ExecPort), oxikube_kube, oxikube_workspace, oxikube_settings, oxikube_theme, oxikube_testkit.
**Depends on:** E03, E04, E05, E06.
**Done when:** (1) `oxikube` opens a local shell tab with the cluster env in <150 ms; (2) exec into a container renders vim/htop correctly at 60 fps with resize; (3) node shell and debug container flows work on kind in the integration suite; (4) a terminal tab can be dragged between panes and bottom dock and survives layout persistence; (5) no credential ever appears in scrollback persistence (test asserts nothing persisted).
**Risks:** alacritty_terminal API churn (0.26) → pin exactly, isolate in `grid.rs`; Linux/Windows PTY + IME differences → CI smoke on Linux, story for manual Linux check; node shell requires privileged pod creation → MutationGuard confirmation + configurable shell image (k9s `shellPod` config).

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E09-S01 | Define `ExecPort` + `TerminalBackend` trait and fakes | S | `ExecPort::{exec, attach, create_debug_container, node_shell}` return `Box<dyn TerminalBackend>`<br>`TerminalBackend { write, resize, output_stream, kill }` async, object-safe<br>`oxikube_testkit::FakeTerminalBackend` scripted echo | oxikube_ports/exec.rs, oxikube_testkit | E02 |
| E09-S02 | `LocalPty` backend via portable-pty with cluster env injection | M | Spawns user shell (settings `terminal.shell`, default `$SHELL`)<br>Env: KUBECONFIG (temp merged file per cluster), KUBE_CONTEXT alias, OXIKUBE_NAMESPACE, PATH untouched<br>Resize + kill + exit code propagate; tested on macOS+Linux | oxikube_terminal/backend/local.rs | S01 |
| E09-S03 | `KubeStream` backend over `AttachedProcess` | M | `Api::<Pod>::exec` with `AttachParams::interactive_tty()`; stdin/stdout piped to backend<br>`TerminalSize` sent on resize; `take_status` surfaces exit status<br>Reconnect prompt on WS close; unit test with FakeResourcePort + integration on kind | oxikube_kube/exec.rs | S01, E04 |
| E09-S04 | alacritty_terminal grid wrapper + event loop bridge | M | `TerminalState` owns `alacritty_terminal::Term`, processes bytes from backend on Tokio, posts batched updates (frame-coalesced) to GPUI<br>Scrollback limit from settings; selection + search over grid<br>Unit tests feed VT sequences and assert grid | oxikube_terminal/grid.rs, oxikube_runtime | S01 |
| E09-S05 | `TerminalElement`: paint cells, cursor, selection, hyperlinks | L | Custom `Element` with request_layout/prepaint/paint using theme terminal colours (16 ANSI + fg/bg)<br>Bold/italic/underline/strike, wide glyphs, cursor styles<br>Hyperlink + path detection (cmd-click opens browser/path)<br>Screenshot test via render_to_image | oxikube_terminal/element.rs, oxikube_theme | S04 |
| E09-S06 | Keystroke → escape sequence mapping + IME | M | Zed-style mapping table (arrows, modifiers, alt as meta option, bracketed paste)<br>IME composition via `EntityInputHandler`<br>Copy on select (setting), paste with confirmation for multi-line (setting) | oxikube_terminal/mappings.rs | S05 |
| E09-S07 | TerminalView as workspace Item + bottom dock panel | M | Implements `Item` (tab title = process/pod name, dirty = running)<br>Draggable between panes/dock; `terminal: new`, `terminal: split`, `terminal: close` commands<br>Layout persistence stores backend descriptor only; restore spawns fresh shell | oxikube_terminal/view.rs, oxikube_workspace | S05, E05 |
| E09-S08 | ExecService: pod shell / attach commands with container picker | M | Commands `pod: shell`, `pod: attach` on selection; picker when >1 container; shell fallback chain (`bash`→`sh`) like Lens<br>MutationGuard: exec in read-only mode configurable (default: blocked in read-only, no confirmation otherwise)<br>Tool stub `pod_exec` registered as "unsafe, interactive, not exposed to agents by default" | oxikube_app/exec.rs, oxikube_resources_ui | S03, S07 |
| E09-S09 | Node shell via privileged shell pod | M | Creates pod from settings template (`nodeShell.image`, nsenter args, tolerations) in configurable ns; waits Ready; exec; deletes on close<br>MutationGuard: blocked in read-only; confirmation "medium" naming node + image; registers Command `node: shell` + tool stub<br>Integration test on kind | oxikube_app/exec.rs, oxikube_kube/node_shell.rs | S08 |
| E09-S10 | Ephemeral debug container | M | `patch_ephemeral_containers` with image/target container/command from a dialog (defaults: busybox, same target)<br>Attaches terminal when container Running<br>MutationGuard: blocked in read-only; confirmation low; Command `pod: debug` + tool stub `pod_debug` | oxikube_kube/debug.rs, oxikube_app, oxikube_resources_ui | S03, S08 |
| E09-S11 | Terminal settings + keymap context | S | Settings: shell, font family/size, scrollback, copy_on_select, cursor shape, bell<br>Keymap context `Terminal`; default bindings for copy/paste/search/clear/new<br>Settings schema entries + hot reload | oxikube_terminal, oxikube_settings, oxikube_keymap | S07 |
| E09-S12 | Reconnect, errors and lifecycle hardening | S | WS drop shows inline banner with "Reconnect"; local shell exit shows exit code and "Restart"<br>Killing a tab aborts backend task (abort-on-drop)<br>Leak test: 50 open/close cycles no task growth | oxikube_terminal | S07 |
| E09-S13 | Terminal integration + screenshot tests | M | kind suite: exec `echo`, resize, attach to a long-running pod, node shell, debug<br>Golden screenshots for colours/bold on macOS nightly | oxikube_testkit, .github/workflows/integration.yml | S09, S10 |

### E10 — Manifest editor  (Phase 1, Area: ui)
**Goal:** A purpose-built YAML/JSON editor (gpui-component `EditorState` + tree-sitter-yaml) for viewing, editing, validating and applying Kubernetes manifests with cluster-OpenAPI schema diagnostics, diff against the live object, server dry-run preview and server-side apply. Text is the source of truth: comments and key order are never re-serialised away.
**In scope:**
- `oxikube_editor` crate: `ManifestEditor` view (Item), read-only mode, line numbers, folding, search/replace, multi-cursor, undo, soft-wrap toggle, managedFields toggle, secrets masking.
- Schema pipeline: fetch `/openapi/v3` per cluster + group-version (cached, invalidated on DiscoveryChanged), merge `$ref`/allOf, validate (unknown fields, types, enums, required, patterns, x-kubernetes-* hints) using `granit-parser` spans → `DiagnosticSet` squiggles.
- Hover docs + completion from schema.
- Diff vs live (comment-insensitive, ignores server-managed fields), three-way merge on "object changed on server" banner.
- Apply flow: server dry-run → diff preview → SSA with field manager `oxikube` (force-conflicts opt-in) → fallback replace; errors mapped to fields.
- Create-from-template (embedded minimal manifests per kind + user templates dir), multi-document paste/apply.
**Out of scope:** general-purpose file/folder editing (not planned); apply directory/kustomize (E20); agent-proposed manifests (E27, uses this editor's diff/apply API); Helm values editor reuses this editor (E16).
**Crates:** oxikube_editor, oxikube_ui (editor wrapper), oxikube_app (ApplyService, TemplateService), oxikube_ports (ResourcePort dry_run/apply, SchemaPort), oxikube_kube (openapi v3 fetch, SSA), oxikube_domain (Manifest, Diagnostic), oxikube_testkit.
**Depends on:** E04, E05, E07 (opens from detail drawer).
**Done when:** (1) editing a Deployment YAML shows schema squiggles within 100 ms of typing on a 2k-line file; (2) dry-run diff preview appears before every apply and read-only clusters block apply; (3) a 5 MB `get -o yaml` dump opens and scrolls without jank (frame >16 ms count logged < 1%); (4) CRD manifests validate against their CRD schema; (5) template creation for all core kinds works end-to-end on kind.
**Risks:** gpui-component editor gaps (inline widgets, soft wrap) → canvas overlays via `EditorState::range_to_bounds`, budget S12 for upstream/vendor; OpenAPI v3 size (tens of MB on big clusters) → lazy per-group fetch + disk cache; SSA conflicts with other managers → explicit conflict dialog listing managers.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E10-S01 | `SchemaPort` + OpenAPI v3 adapter with cache | M | `SchemaPort::schema_for(gvk) -> Arc<JsonSchema>` resolves `$ref`/allOf into a flattened view<br>Fetch `/openapi/v3` index then per-group paths lazily; disk cache keyed by cluster+serverVersion; invalidated on DiscoveryChanged<br>Unit tests with fixture schemas incl. a CRD with x-kubernetes-preserve-unknown-fields | oxikube_ports/schema.rs, oxikube_kube/openapi.rs, oxikube_domain/schema.rs | E04 |
| E10-S02 | Spanned YAML model with granit-parser | M | Parse buffer into node tree with byte spans + JSON path per node; multi-doc support (`---`)<br>Error recovery yields partial tree + syntax diagnostics<br>Property tests round-trip paths | oxikube_editor/yaml/parse.rs | — |
| E10-S03 | Schema validator → diagnostics | M | Validates unknown fields, wrong types, enums, required, patterns, int-or-string, x-kubernetes-* hints<br>Produces `Diagnostic{span, severity, message, code}`; <50 ms on 2k lines<br>Fixture-based tests for Pod/Deployment/CRD cases | oxikube_editor/yaml/validate.rs | S01, S02 |
| E10-S04 | `ManifestEditor` view on gpui-component Editor | L | Wrapper in oxikube_ui exposes our `EditorApi` trait (text, selections, decorations, diagnostics) so views never import gpui-component<br>Line numbers, folding, search/replace, multi-cursor, undo, read-only, soft-wrap toggle<br>Squiggles via `DiagnosticSet`; gutter markers + end-of-line messages via canvas overlay<br>Keymap context `ManifestEditor` | oxikube_ui/editor.rs, oxikube_editor/view.rs | S03, E05 |
| E10-S05 | Hover docs and completion from schema | M | Hover shows field description/type/enum; completion lists sibling fields + enum values; snippets for common blocks (container, volume)<br>Works inside arrays and CRDs | oxikube_editor/intel.rs | S03, S04 |
| E10-S06 | Open-from-resource with masking and managedFields toggle | S | "Edit"/"View YAML" from any resource opens editor tab titled `kind/name`<br>Secret `data` masked by default (toggle reveals, never persisted decoded)<br>managedFields hidden by default; `status` dimmed; kubectl-like key ordering on render | oxikube_editor/render.rs, oxikube_resources_ui | S04, E07 |
| E10-S07 | Diff vs live + "changed on server" banner | M | Comment-insensitive line diff (similar crate) ignoring managedFields/resourceVersion/status<br>Live watch on the edited object; banner offers Merge (3-way) / Reload / Keep mine; clean buffers reload silently<br>Side-by-side and inline diff view in bottom panel | oxikube_editor/diff.rs | S04, E04 |
| E10-S08 | ApplyService: dry-run → preview → SSA → fallback | M | `ResourcePort::apply(gvk, json, DryRun)` then diff preview modal; SSA field manager `oxikube`; 415/unsupported → replace with resourceVersion; conflicts listed with managers + force option<br>Server errors mapped to field spans<br>MutationGuard: blocked in read-only; confirmation "low" (preview is the confirmation); audit record; Command `manifest: apply`; tool stub `apply_manifest` (gated) | oxikube_app/apply.rs, oxikube_kube/apply.rs | S07 |
| E10-S09 | Create from template + multi-document apply | M | Templates embedded per core kind + `~/.config/oxikube/templates/*.yaml`; "Create resource" picker → editor with template + namespace prefilled<br>Multi-doc buffers apply in order with per-doc results panel<br>MutationGuard same as S08; Command `resource: create` | oxikube_app/templates.rs, oxikube_editor/templates/, oxikube_resources_ui | S08 |
| E10-S10 | JSON mode + YAML⇄JSON convert | S | `.json` detection, tree-sitter-json highlighting, same validation pipeline<br>Command `manifest: convert to json/yaml` preserving comments where possible (warn on loss) | oxikube_editor | S04 |
| E10-S11 | Large-file and performance hardening | M | Benchmarks: 5 MB YAML open <500 ms, typing latency <16 ms with validation debounced (150 ms)<br>Validation runs on background executor, cancellable<br>Virtualised diagnostics gutter | oxikube_editor, xtask bench | S03, S04 |
| E10-S12 | Editor settings + schema entries | S | Settings: tab size, font, soft wrap, show managedFields, mask secrets, validation on/off, field manager name<br>Hot reload applies live | oxikube_editor, oxikube_settings | S04 |
| E10-S13 | Editor integration tests on kind | M | Apply new ConfigMap from template, edit live Deployment replicas with SSA, conflict scenario with kubectl manager, CRD instance validation | oxikube_testkit, integration.yml | S08, S09 |


### E11 — Command palette, ':' jump & keymaps  (Phase 1, Area: ui)
**Goal:** Make every action discoverable and rebindable: a Zed-style command palette over the command bus, a k9s-style `:` quick-jump with aliases and filters, `/` table filtering grammar, keymap contexts per view, and an optional vim keymap. Keyboard parity with k9s without losing mouse discoverability.
**In scope:**
- `oxikube_palette`: Picker (vendored Zed PickerDelegate design under GPL), CommandPalette (all registered Commands with keybinding hints, recents), `:` jump bar (`:pods`, `:deploy kube-system`, `:pod /re`, `:pod app=x`, `:ctx name`, `:ns`, `:helm`, `:pf`, `:ev`, `:crd`, aliases incl. shortnames/kinds from discovery), `/` filter grammar (`/re`, `/!re`, `/-l selector`, `/-f fuzzy`).
- `oxikube_keymap` extensions: per-view key contexts (`Workspace`, `ResourceTable`, `DetailDrawer`, `LogView`, `Terminal`, `ManifestEditor`, `Palette`), per-OS defaults, user `keymap.json` layering, `vim.json` optional base keymap, binding validator, conflict report.
- SearchService in `oxikube_app`: alias table (`aliases.json` user file + discovery-derived), fuzzy matching (nucleo-matcher), command history.
- Help overlay (`?`) listing active context bindings like k9s.
**Out of scope:** GUI keymap editor (E21); cross-cluster palette search (E29); agent slash commands (E27).
**Crates:** oxikube_palette, oxikube_keymap, oxikube_app (SearchService, CommandBus metadata), oxikube_workspace, oxikube_settings, oxikube_domain (Command metadata).
**Depends on:** E05, E06, E07.
**Done when:** (1) every Command registered in the bus appears in the palette with its binding and runs; (2) `:deploy kube-system`, `:pod app=nginx`, `:ctx prod` navigate correctly incl. CRD aliases; (3) user keymap overrides work with hot reload and invalid bindings surface as notifications; (4) vim keymap gives j/k/gg/G/`/`/`:` in tables; (5) `?` shows the active context's bindings.
**Risks:** Key-context precedence bugs → vendor Zed's dispatch-tree tests; alias collisions between CRDs → discovery-derived aliases lower priority than built-ins, conflict list in help.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E11-S01 | Command metadata + registry introspection | S | `Command` carries id, title, category, keymap action name, availability predicate (context, read-only, selection kind)<br>`CommandBus::list(ctx)` returns runnable commands; unit tests | oxikube_domain/command.rs, oxikube_app/command_bus.rs | E05 |
| E11-S02 | Generic Picker component (Zed PickerDelegate pattern, vendored) | M | `Picker<D: PickerDelegate>` with fuzzy input, virtual list, selected index, confirm/secondary confirm, dismiss<br>GPL header crediting Zed; adapted to oxikube_ui tokens<br>gpui tests: keystrokes navigate + confirm | oxikube_palette/picker.rs | E05 |
| E11-S03 | Command palette view | M | `cmd-shift-p`/`ctrl-shift-p` opens; lists S01 commands with bindings, categories, recents first; executes via bus<br>Unavailable commands hidden (not greyed) unless "show all" toggle<br>Keymap context `Palette` | oxikube_palette/command_palette.rs | S01, S02 |
| E11-S04 | Alias table + discovery-derived resource aliases | M | Built-in k9s aliases (po, dp, svc, sts, ds, cj, ing, np, pv, pvc, sc, cm, sec, sa, ro, rb, cr, crb, hpa, pdb, ev, no, ns, crd…)<br>Discovery adds plural/singular/shortNames/Kind per GVR with lower priority<br>User `aliases.json` (name → GVR or command) hot-reloaded; conflicts reported | oxikube_app/search/aliases.rs, oxikube_settings | E07 |
| E11-S05 | `:` jump bar grammar + parser | M | Grammar: `:<alias> [ns] [/filter] [k=v,..] [@ctx]`, `:ctx [name]`, `:ns`, `:q`<br>Parser with tests for all forms incl. errors; history with `-`, `[`, `]`<br>Executes navigation Commands (open table for GVR in ns with filter) | oxikube_app/search/jump.rs, oxikube_palette/jump_bar.rs | S04 |
| E11-S06 | `/` filter grammar for tables | S | `/re`, `/!re`, `/-l key=value`, `/-f fuzzy`; label selectors go server-side (watcher label_selector) others client-side<br>Filter state persisted per view; `esc` clears; n/N match navigation in logs/YAML | oxikube_app/search/filter.rs, oxikube_resources_ui | S05 |
| E11-S07 | Keymap contexts for all Phase 1 views + per-OS defaults | M | Contexts: Workspace, ClusterTab, ResourceTable, DetailDrawer, LogView, Terminal, ManifestEditor, Palette, JumpBar<br>`default-macos.json`, `default-linux.json` with k9s-inspired verbs (y, d, e, ctrl-d, l, s, f, shift-f, ctrl-w…)<br>Dispatch tests vendored/adapted from Zed | oxikube_keymap/defaults/, oxikube_keymap/dispatch.rs | E05 |
| E11-S08 | User keymap.json layering + validator + hot reload | M | `~/.config/oxikube/keymap.json` sections `{context, bindings}` with `null` to unbind; validated (unknown action, bad keystroke) → notification with line numbers<br>notify-based reload; `keymap: open user keymap` command creates file with template | oxikube_keymap/user.rs | S07 |
| E11-S09 | Vim base keymap | M | `vim.json` base keymap selectable in settings (`base_keymap: "vim"`): j/k/gg/G/ctrl-d/u in tables, `/` search, `:` jump, `dd`=delete with confirmation, `yy`=copy name<br>Does not alter terminal/editor insert behaviour | oxikube_keymap/defaults/vim.json | S07 |
| E11-S10 | Help overlay (`?`) and binding hints | S | Modal listing active context bindings grouped by category; searchable; shows which bindings are user-overridden<br>Hover/tooltips on buttons show bindings | oxikube_palette/help.rs | S07 |
| E11-S11 | Fuzzy matching service + recents persistence | S | nucleo-matcher with ranking tests; command recents and jump history stored via StatePort<br>Latency <5 ms for 2k candidates | oxikube_app/search/fuzzy.rs, oxikube_state_sqlite | S03 |
| E11-S12 | Palette/jump integration tests | S | gpui tests: open palette, type, confirm; `:pods kube-system` opens table with ns; vim keymap navigation | oxikube_testkit | S05, S09 |


### E12 — Per-kind panels & actions  (Phase 2, Area: ui)
**Goal:** Reach Lens + k9s parity for every built-in kind: specialised list columns, structured detail panels and the full action set (scale, restart, rollout history/undo, trigger/suspend, evict, cordon/drain, decode/edit secrets, set image, etc.) on top of the generic browser from E07. Each action is a Command with MutationGuard policy and an MCP tool stub.
**In scope:** Sidebar sections Nodes, Workloads (Overview, Pods, Deployments, DaemonSets, StatefulSets, ReplicaSets, ReplicationControllers, Jobs, CronJobs), Config (ConfigMaps, Secrets, ResourceQuotas, LimitRanges, HPA, VPA, PDB, PriorityClasses, RuntimeClasses, Leases, Mutating/Validating webhooks, ValidatingAdmissionPolicy/Binding), Network (Services, Endpoints, EndpointSlices, Ingresses, IngressClasses, NetworkPolicies, Gateway API: GatewayClass/Gateway/HTTPRoute/GRPCRoute/TLSRoute), Storage (PVC, PV, StorageClass, VolumeSnapshot/Class if present), Namespaces, Custom Resources (per-group sidebar entries, printer columns via Table API, generic detail), CRDs view; Pod containers drill-down; UsedBy/references; workloads overview page.
**Out of scope:** RBAC views (E17); Events (E14); metrics columns/charts (E13 supplies `MetricsService`, this epic consumes via column provider); Helm (E16); logs/exec (E08/E09); bulk ops framework exists in E07 (this epic adds per-kind bulk actions); file transfer & copy-to-cluster (E20).
**Crates:** oxikube_resources_ui (kinds/*), oxikube_app (WorkloadService, NodeService, SecretService, RolloutService), oxikube_domain (view-models per kind), oxikube_kube (subresource calls), oxikube_ports, oxikube_testkit.
**Depends on:** E04, E07, E10, E11; E13 for metrics columns (soft).
**Done when:** (1) every kind in the Freelens catalog (report-features §A1) has columns + detail panel + listed actions; (2) k9s verbs mapped (`s` scale/shell, `r` restart/drain, `t` trigger, `z` sanitize, `u` used-by, `x` decode, `i` set image, `ctrl-l` rollback, `c`/`u` cordon); (3) all mutations respect read-only mode and confirmation levels in tests; (4) CRDs with printer columns render identical columns to `kubectl get`; (5) integration suite exercises scale/restart/rollback/cordon/drain/evict/trigger/suspend on kind.
**Risks:** Breadth → stories grouped so teams parallelise by section; drain semantics (PDB, DaemonSets, emptyDir) → port kubectl drain algorithm with tests; Gateway API optional CRDs → detect via discovery.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E12-S01 | Kind registry + per-kind `KindSpec` (columns, detail, actions, templates) | M | `KindSpec { gvk, columns: ColumnProvider, detail: DetailRenderer, actions: Vec<CommandId>, sidebar: Section }` registered via `init(cx)` per module<br>Unknown kinds fall back to generic spec (E07)<br>Sidebar builds from registry + discovery presence + RBAC `SelfSubjectRulesReview` gating hook (impl in E17) | oxikube_resources_ui/kinds/mod.rs, oxikube_app/kinds.rs | E07 |
| E12-S02 | Pods: columns, status derivation, containers drill-down | L | Columns per Lens/k9s: Name, NS, Ready, Status, Restarts (+last restart), CPU/Mem (from E13 provider, "–" when absent), %CPU/R..L, IP, Node, QoS, SA, Controlled By, Age; default-hidden set<br>Status derivation ported fresh from kubectl printer (Init:N/M, CrashLoopBackOff, Terminating…) with table tests (do not copy kdash's)<br>Containers sub-table: image, state, restarts, probes, ports, resources | oxikube_domain/pod.rs, oxikube_resources_ui/kinds/pod.rs | S01 |
| E12-S03 | Pod detail panel + actions | M | Panel: metadata, conditions, containers (env resolved from CM/Secret refs with masking, mounts, probes), volumes, tolerations/affinity, owner chain, node link<br>Actions: Logs, Shell, Attach, Port-forward, Evict (`Api::evict`), Delete, Force delete (grace 0), Sanitize (delete Completed/Failed in view), Jump to owner, Show node<br>MutationGuard: evict/delete confirm "medium" (name), force delete "high" (type name); Commands + tool stubs `pod_evict`, `pod_delete` | oxikube_resources_ui/kinds/pod.rs, oxikube_app/workloads.rs | S02 |
| E12-S04 | Deployments/StatefulSets/ReplicaSets/RCs: columns, detail, scale | M | Columns Ready/Up-to-date/Available/Replicas/Age + conditions; detail shows pod template, strategy, owned RS/pods, selector<br>Scale dialog via `patch_scale` with current/desired; MutationGuard: blocked read-only, confirm "low" (scale to 0 = "medium")<br>Commands `workload: scale`; tool stub `scale_workload` | kinds/deployment.rs, statefulset.rs, replicaset.rs, oxikube_app/workloads.rs | S01 |
| E12-S05 | Restart (rollout restart) for Deploy/DS/STS + set image | M | Restart = merge patch `kubectl.kubernetes.io/restartedAt` (kdash ResourcePatch builder reused); Set image dialog per container (k9s `i`)<br>MutationGuard: confirm "low"; Commands `workload: restart`, `workload: set image`; tool stubs | oxikube_app/workloads.rs, kinds/* | S04 |
| E12-S06 | Rollout history + undo | M | History from owned ReplicaSets (`deployment.kubernetes.io/revision`), change-cause; undo patches pod template from chosen RS (kubectl rollout undo algorithm); STS/DS via ControllerRevisions<br>MutationGuard: confirm "medium"; Command `workload: rollback`; tool stub `rollback_workload`; kind integration test | oxikube_app/rollout.rs, kinds/deployment.rs | S04 |
| E12-S07 | DaemonSets + Workloads Overview page | M | DS columns (Desired/Current/Ready/Up-to-date/Available/Node selector); overview tiles per workload kind with status bars + recent warnings (events from E14 hook) | kinds/daemonset.rs, oxikube_resources_ui/overview.rs | S04 |
| E12-S08 | Jobs & CronJobs: trigger, suspend/resume | M | Job columns Completions/Duration/Conditions; CronJob Schedule/Timezone/Suspend/Active/Last schedule<br>Trigger = create Job from jobTemplate (kdash `trigger_cronjob` reused, generateName + ownerRef); Suspend/Resume merge patch<br>MutationGuard: confirm "low"; Commands `cronjob: trigger/suspend`, `job: suspend`; tool stubs | kinds/job.rs, cronjob.rs, oxikube_app/workloads.rs | S01 |
| E12-S09 | Nodes: columns, detail, cordon/uncordon | M | Columns Roles/Status/Version/OS/Kernel/Internal-IP/Pods/CPU%/Mem%/Taints/Schedulable/Age (+ instance type labels)<br>Detail: conditions, capacity/allocatable with allocation bars (requests/limits via pods on node), taints, images, pods list<br>Cordon/uncordon = `spec.unschedulable` patch; MutationGuard confirm "low"; Commands + tool stubs | kinds/node.rs, oxikube_app/nodes.rs | S01 |
| E12-S10 | Node drain | M | kubectl drain algorithm: cordon → list pods on node → skip DaemonSet pods, mirror pods; options ignore-daemonsets, delete-emptydir-data, force, grace, timeout → evict with PDB retry/backoff; progress UI with per-pod status; cancel<br>MutationGuard: confirm "high" (type node name); Command `node: drain`; tool stub `drain_node`; kind integration test with a PDB | oxikube_app/nodes/drain.rs | S09 |
| E12-S11 | ConfigMaps + Secrets: view/decode/edit with transparent base64 | M | CM detail key/value with syntax-aware preview; Secret detail masked with reveal (x), decode-all toggle in YAML view, type-aware (docker config, TLS)<br>"Edit decoded" opens editor with plaintext values and re-encodes on apply (never writes decoded to disk)<br>MutationGuard: edit confirm "low"; Create-secret dialog (opaque/docker/tls); Commands + tool stubs (`get_secret` tool returns masked by default) | kinds/configmap.rs, secret.rs, oxikube_app/secrets.rs | S01, E10 |
| E12-S12 | Config section remainder | M | ResourceQuota (used/hard bars), LimitRange, HPA v2 (metrics/targets/current, scale targets), VPA (if CRD), PDB (allowed disruptions), PriorityClass, RuntimeClass, Lease, Mutating/ValidatingWebhookConfiguration, ValidatingAdmissionPolicy/Binding — columns + detail each | kinds/config/*.rs | S01 |
| E12-S13 | Services, Endpoints, EndpointSlices | M | Service columns Type/ClusterIP/ExternalIP/Ports/Selector/Status; detail lists endpoint slices, backing pods, "Open in browser" per port (via port-forward E15 hook)<br>UsedBy navigation to pods via selector | kinds/service.rs, endpoints.rs | S01 |
| E12-S14 | Ingress, IngressClass, NetworkPolicy, Gateway API | M | Ingress rules table + LB addresses, IngressClass "set default" (annotation patch, confirm "low"), NetworkPolicy ingress/egress rule rendering<br>Gateway API kinds detected via discovery: GatewayClass, Gateway (listeners/addresses), HTTPRoute/GRPCRoute/TLSRoute (hostnames, rules, parents) | kinds/network/*.rs | S01 |
| E12-S15 | PVC, PV, StorageClass, VolumeSnapshots | M | PVC columns StorageClass/Size/Status/Pods(UsedBy); PV Capacity/AccessModes/Reclaim/Claim; SC provisioner/default badge + "set default"; VolumeSnapshot/Class when CRDs exist<br>PVC resize action (patch requests.storage, confirm "medium") | kinds/storage/*.rs | S01 |
| E12-S16 | Namespaces: list, create, delete, favourites, use | M | Columns Status/Labels/Age/pod count; Create dialog; Delete with "type the name" (high); handles HNC subnamespaces warning<br>"Use namespace" sets session selection; favourites + number keys via keymap | kinds/namespace.rs, oxikube_app/namespaces.rs | S01, E06 |
| E12-S17 | CRDs view + Custom Resources sidebar groups | M | CRD list (Group/Versions/Scope/ShortNames/Age) with detail (schema tree, versions, printer columns, conditions); Enter opens CR list<br>Sidebar "Custom Resources" grouped by API group (collapsible, searchable); CR lists via Table API printer columns; generic detail + YAML/edit/delete | kinds/crd.rs, oxikube_resources_ui/sidebar.rs | S01, E04 |
| E12-S18 | UsedBy / references for SA, CM, Secret, PVC, PriorityClass | S | Reverse index built from pod specs in ResourceStore; "Used by" tab lists referencing workloads; navigation | oxikube_app/references.rs | S03, S11 |
| E12-S19 | Per-kind bulk actions + multi-select wiring | S | Bulk delete/restart/label/annotate on selected rows with per-row results; label editor dialog<br>MutationGuard aggregates to highest level among items; Commands `selection: delete/restart/label` | oxikube_resources_ui/bulk.rs | S03, S05 |
| E12-S20 | Section integration + screenshot test matrix | M | kind suite seeding every kind (fixtures in testkit) and asserting columns/actions; golden screenshots for 6 detail panels nightly | oxikube_testkit/fixtures, integration.yml | S02–S17 |

### E13 — Metrics & cluster overview  (Phase 2, Area: adapters+ui)
**Goal:** Live resource usage from metrics-server in every table and detail panel, history graphs when a Prometheus-compatible endpoint is reachable (auto-detected), and a Lens-style cluster overview dashboard plus k9s-style pulses. Metrics absence is a visible state, never a swallowed error.
**In scope:**
- `MetricsPort` adapter on `k8s-metrics` (`NodeMetrics`, `PodMetrics`) with polling cadence per cluster, utilisation maths (requests/limits vs usage; Quantity parser from E02), GPU resources via configurable vendors.
- `PromqlPort` adapter: HTTP client, auto-detect providers (kube-prometheus/operator, Lens stack, VictoriaMetrics, Mimir, OpenShift) by probing services; manual URL/bearer/path per cluster; queries for cluster/node/pod/PVC/ingress CPU/memory/network/disk.
- MetricsService in app: column provider (CPU/Mem/%R/%L), sparkline ring buffers, chart data series.
- Overview page: node/pod health, capacity, allocation bars, top consumers, recent warnings (E14), version/distribution detection; Pulses view with gauges + sparklines per kind.
- Charts via gpui-component chart wrapped in `oxikube_ui`.
**Out of scope:** installing a metrics stack into clusters (never); alerting (E29); per-resource events (E14).
**Crates:** oxikube_ports (MetricsPort, PromqlPort), oxikube_kube (metrics), oxikube_prometheus, oxikube_app (MetricsService), oxikube_overview_ui, oxikube_ui (charts), oxikube_resources_ui (columns), oxikube_settings.
**Depends on:** E04, E07, E12 (column slots).
**Done when:** (1) pod/node tables show CPU/Mem when metrics-server exists and a clear "metrics unavailable" badge otherwise; (2) detail panels show 1h graphs when Prometheus auto-detect succeeds on kube-prometheus; (3) overview renders capacity/allocation for a 100-node fixture under 16 ms/frame; (4) VictoriaMetrics endpoint works via manual config; (5) metrics polling is per-cluster, pauses when the tab is hidden.
**Risks:** k8s-metrics single-maintainer → internal fallback types behind the port; Prometheus query drift across stacks → provider-specific query sets with fixture tests; polling load on big clusters → adaptive interval + only visible kinds.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E13-S01 | `MetricsPort` + metrics-server adapter | M | `MetricsPort::{node_metrics, pod_metrics(ns)}` using `Api::<k8s_metrics::v1beta1::PodMetrics>`; absence (404/503) → `MetricsState::Unavailable(reason)`<br>Internal fallback types if crate breaks; unit tests with fixtures | oxikube_ports/metrics.rs, oxikube_kube/metrics.rs | E02, E04 |
| E13-S02 | Utilisation maths + Quantity | M | Requests/limits aggregation per pod/node/namespace; %CPU/R, %CPU/L, %Mem/R, %Mem/L; GPU vendors from settings (`metrics.gpu_vendors`)<br>Property tests against kubectl-view-allocations semantics (CC0 reference) | oxikube_domain/metrics.rs, oxikube_app/metrics.rs | S01 |
| E13-S03 | MetricsService polling + column provider | M | Per-cluster poller (default 15 s, adaptive on error), paused when no visible consumer; ring buffer last 60 samples<br>`ColumnProvider` injects CPU/Mem/% columns into Pod/Node/Container specs with sort + warn/critical thresholds colouring (k9s thresholds setting) | oxikube_app/metrics.rs, oxikube_resources_ui | S02, E12 |
| E13-S04 | `PromqlPort` + HTTP client adapter | M | Instant + range queries, bearer/basic/none auth, custom path prefix, TLS/insecure toggle, timeout; error taxonomy<br>Unit tests against recorded responses | oxikube_ports/promql.rs, oxikube_prometheus/client.rs | E02 |
| E13-S05 | Provider auto-detect + per-cluster metrics settings | M | Probe services (`prometheus-operated`, `kube-prometheus-stack-prometheus`, `prometheus` in `lens-metrics`, `vmsingle`/`vmselect`, `mimir-query-frontend`, OpenShift thanos-querier) via service proxy or port-forward<br>Settings per cluster: provider (auto/manual), url, auth, enabled; detection result shown in cluster settings | oxikube_prometheus/detect.rs, oxikube_settings | S04, E06 |
| E13-S06 | Query catalogue per provider | M | Query sets for cluster/node/pod/container/PVC/ingress/namespace: cpu usage/requests/limits, memory, network rx/tx, disk, pod count; provider variants (label names differ)<br>Fixture tests for each | oxikube_prometheus/queries/*.rs | S05 |
| E13-S07 | Chart components in oxikube_ui | M | Line/area/bar wrappers over gpui-component charts with theme tokens, time axis, hover tooltip, legend, "no data" state; sparkline mini-chart<br>Screenshot tests | oxikube_ui/charts.rs | E05 |
| E13-S08 | Detail-panel graphs (Pod/Node/Deployment/Namespace/PVC/Ingress) | M | Time-range selector (15m/1h/6h/24h), per-container series, requests/limits lines; graceful hide when no PromQL | oxikube_resources_ui/metrics_panel.rs | S06, S07 |
| E13-S09 | Cluster overview page | L | Landing view per cluster tab: health tiles (nodes ready, pods by phase), capacity + allocation bars, top CPU/Mem pods & namespaces, warnings feed (E14 hook), cluster info (version, distribution detection, node count)<br>Works with metrics-server only (no graphs) and with Prometheus (graphs) | oxikube_overview_ui/overview.rs | S03, S07 |
| E13-S10 | Pulses view | M | Gauges + sparklines per kind (nodes, ns, pods, deploys, sts, ds, jobs, cj, pv, pvc, hpa, ing, np, sa) + cluster CPU/Mem; keyboard nav drills into tables (`:pulses`) | oxikube_overview_ui/pulses.rs | S03 |
| E13-S11 | Node/namespace allocation views | S | Node detail allocation bars (requests/limits/usage per resource); Namespace detail aggregated usage vs quotas | oxikube_resources_ui/kinds/node.rs, namespace.rs | S02 |
| E13-S12 | Metrics tools + context providers | S | Tool stubs `top_pods`, `top_nodes`, `query_promql` (read-only) registered; `@metrics/pod/ns/name` context provider renders last samples as text | oxikube_app/tools/metrics.rs | S03, S06 |
| E13-S13 | Metrics integration tests | M | kind + metrics-server addon: tables populate; kube-prometheus-stack fixture in nightly: auto-detect + graphs | oxikube_testkit, nightly.yml | S05, S09 |

### E14 — Events & notifications  (Phase 2, Area: app+ui)
**Goal:** A cluster-wide events view with rich filtering, per-resource events everywhere, an in-app notification centre for Warning events on watched resources, opt-in OS notifications, and warning badges in tables, all fed by one events watcher per cluster.
**In scope:**
- EventService: watcher on `events.k8s.io/v1` with core/v1 fallback, ring buffer (size setting), dedupe by UID + count/series, index by involvedObject.
- Events view (`:ev`): columns Type/Reason/Object/Source/Count/Age/Last seen/Message; filters type/reason/kind/namespace/text; faults-only toggle (`ctrl-z`); live tail; copy/export.
- Per-resource Events tab in detail drawer; warning icon column in tables.
- Notification centre panel: toasts + list, per-cluster mute, rules (Warning events on selected kinds, port-forward drops, agent needs permission, update available); OS notifications via `NotifierPort` (opt-in).
**Out of scope:** alert-manager integration (E29); audit log UI (E19); metrics-based alerts.
**Crates:** oxikube_app (EventService, NotificationService), oxikube_ports (NotifierPort), oxikube_kube (events watch), oxikube_notify_os, oxikube_events_ui, oxikube_workspace (notification panel), oxikube_resources_ui (warning column), oxikube_settings, oxikube_state_sqlite (mute state).
**Depends on:** E04, E05, E07.
**Done when:** (1) events view updates live on kind within 1 s of `kubectl run` failures; (2) filters and faults-only work and persist per cluster; (3) every detail drawer shows the object's events sorted by last seen; (4) a Warning event on a watched Deployment raises a toast + OS notification when enabled, muted per cluster when muted; (5) memory bounded: 10k events ring buffer < 50 MB.
**Risks:** events.k8s.io vs core/v1 field differences (regarding/involvedObject, series) → normalised domain `Event`; notification spam → rate limiting + grouping by reason.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E14-S01 | Domain `Event` normalisation + events watcher | M | Map `events.k8s.io/v1` (regarding, series.count, lastObservedTime) and `core/v1` (involvedObject, count, lastTimestamp) into one `Event`<br>Watcher via kube-runtime with fallback when events.k8s.io absent; tests with both fixtures | oxikube_domain/event.rs, oxikube_kube/events.rs | E04 |
| E14-S02 | EventService ring buffer + index | M | Per-cluster buffer (setting `events.buffer`, default 10k), dedupe by UID updating count, index by involvedObject UID and by namespace; query API with filters<br>Bench: insert 10k events < 50 ms | oxikube_app/events.rs | S01 |
| E14-S03 | Events view | M | Table columns + filters (type, reason, kind, namespace, text regex), faults-only toggle, live tail with pause, sort by last seen<br>`:ev`, keymap context `EventsView`; copy row, export JSON | oxikube_events_ui/view.rs | S02, E07 |
| E14-S04 | Per-resource Events tab + warning badges | S | Detail drawer Events tab from index; warning icon column in all tables when object has Warning events in last 1h (setting)<br>Workloads overview recent-warnings panel hook | oxikube_resources_ui/events_tab.rs | S02 |
| E14-S05 | NotificationService + centre panel | M | Rules engine (kinds to watch, reasons ignore-list, rate limit per reason/object); toast + persistent list with read/unread, click-to-navigate<br>Per-cluster mute persisted; keymap context; `notifications: toggle panel` | oxikube_app/notifications.rs, oxikube_workspace/notifications.rs | S02, E05 |
| E14-S06 | `NotifierPort` + OS notification adapter | S | macOS (UNUserNotificationCenter via notify-rust/mac-notification-sys) and Linux (D-Bus); opt-in setting; click focuses app + navigates<br>Fake notifier in testkit | oxikube_ports/notifier.rs, oxikube_notify_os | E02 |
| E14-S07 | Other notification sources wired | S | Port-forward drop (E15 hook), update available (E24 hook), agent permission needed (E27 hook) go through the same service; dedupe keys | oxikube_app/notifications.rs | S05 |
| E14-S08 | Events tools + context provider | S | Tool stub `get_events(ns, involved?, since)` read-only; `@events/kind/ns/name` provider; "Send events to agent" action | oxikube_app/tools/events.rs | S02 |
| E14-S09 | Events settings + schema | S | buffer size, warning badge window, watched kinds, OS notifications toggle, rate limits; hot reload | oxikube_settings | S05 |
| E14-S10 | Events integration tests | S | kind: failing image pull produces Warning → appears in view, badge, toast; mute suppresses; events.k8s.io disabled fixture uses core/v1 | oxikube_testkit, integration.yml | S03, S05 |


### E15 — Port-forward manager  (Phase 2, Area: app+ui)
**Goal:** In-process port forwarding (kube `Portforwarder`, no kubectl) to pods and services with a persistent manager panel across clusters: auto-restart when the backing pod churns, open-in-browser, saved favourites restored on launch, k9s-style annotation auto-forwards.
**In scope:**
- `PortForwardPort` adapter: `Api::<Pod>::portforward` + local TCP listener (tokio) per forward, bind address setting (127.0.0.1 default), random or fixed local port, bytes/conn counters, error channel.
- Service targets resolved to a ready pod via endpoint slices; re-resolve on pod deletion/replacement (watch) with backoff.
- Manager panel (dock): list across clusters with status, ports, URL, traffic; start/stop/restart/remove; favourites (persisted via StatePort) with auto-start on cluster connect; start dialog from Pod/Service detail and container ports.
- `k9scli.io/auto-port-forwards` / `k9scli.io/port-forwards` annotations honoured (opt-in setting).
**Out of scope:** benchmarks (`hey`) (E29); web views (never: open system browser); tunnels to argocd-server (E25 uses this port directly).
**Crates:** oxikube_ports (PortForwardPort), oxikube_kube (portforward), oxikube_app (PortForwardManager), oxikube_portforward_ui, oxikube_state_sqlite, oxikube_settings, oxikube_testkit.
**Depends on:** E04, E05, E06, E12 (Service/Pod detail hooks).
**Done when:** (1) forward to a pod and a service works on kind and survives pod replacement (<3 s reconnect); (2) panel shows all forwards across two connected clusters; (3) favourites restart on launch; (4) port conflicts and permission errors surface inline with suggestions; (5) 50 concurrent forwards keep the UI at 60 fps.
**Risks:** WebSocket port-forward stream multiplexing bugs → one Portforwarder per connection with abort-on-drop; privileged ports on Linux → detect and suggest ≥1024.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E15-S01 | `PortForwardPort` + kube adapter | M | `start(target: PodRef, remote_port, bind, local_port) -> ForwardHandle{local_addr, stats, errors, stop}`; per-connection `Portforwarder::take_stream`<br>Error taxonomy (pod gone, port closed, bind failed); unit tests with fake + kind test | oxikube_ports/portforward.rs, oxikube_kube/portforward.rs | E04 |
| E15-S02 | Local listener + connection multiplexing | M | tokio TcpListener; each accepted conn opens a new portforward stream; counters bytes in/out, active conns; graceful stop closes conns<br>Bench 100 conns | oxikube_kube/portforward.rs | S01 |
| E15-S03 | PortForwardManager service | M | Registry of forwards keyed by id; targets Pod or Service (resolve via EndpointSlice to ready pod, prefer same as before); watch target, auto-restart with backoff on churn; events to NotificationService<br>Unit tests for restart state machine | oxikube_app/portforward.rs | S01, E14 |
| E15-S04 | Start dialog from Pod/Service/container ports | S | Pod detail container ports and Service ports show "Forward" buttons; dialog: remote port, local port (auto/fixed), bind address, name, favourite checkbox<br>Command `portforward: start`; not a mutation (no guard) but blocked when cluster read-only setting `portforward.allowed_in_readonly=false` | oxikube_portforward_ui/dialog.rs, oxikube_resources_ui | S03, E12 |
| E15-S05 | Manager panel | M | Dock panel listing forwards across clusters: cluster, target, ports, status, traffic, uptime; actions start/stop/restart/remove/copy URL/open in browser<br>Keymap context `PortForwards`; `:pf` jump | oxikube_portforward_ui/panel.rs, oxikube_workspace | S03 |
| E15-S06 | Favourites + restore on launch | S | Persist favourites via StatePort (cluster, target, ports, bind); auto-start when cluster session becomes Ready; failures notify, don't block connect | oxikube_app/portforward.rs, oxikube_state_sqlite | S03, E06 |
| E15-S07 | Annotation auto-forwards (k9s compat) | S | Parse `k9scli.io/auto-port-forwards` and `k9scli.io/port-forwards` on pods; opt-in setting; shown as suggestions in dialog | oxikube_app/portforward.rs | S04 |
| E15-S08 | Settings + tools | S | Settings: default bind address, allowed in read-only, port range; tool stubs `list_port_forwards`, `start_port_forward` (gated, agent permission) | oxikube_settings, oxikube_app/tools | S05 |
| E15-S09 | Integration tests | S | kind: forward to nginx pod, curl through it, delete pod → forward recovers; service forward; favourite restored after restart of app state | oxikube_testkit, integration.yml | S03, S06 |

### E16 — Helm  (Phase 2, Area: adapters+ui)
**Goal:** Lens-parity Helm management without reimplementing chart rendering: releases/history/values/manifests/notes read natively from release Secrets (and ConfigMaps), while repos, charts, install, upgrade, rollback and uninstall go through the user's `helm` binary via a `HelmPort`. Everything hides cleanly when `helm` is absent.
**In scope:**
- Native release store reader: `sh.helm.release.v1.*` Secrets/ConfigMaps (base64→base64→gzip→JSON), latest revision per release, history, values (user + computed), rendered manifest, NOTES, resources list (parsed manifest → live objects via ResourceStore), status.
- `HelmCli` adapter: detect binary + version, `repo list/add/remove/update`, `search repo`, `show chart/readme/values`, `template`, `install`, `upgrade`, `rollback`, `uninstall`, `-o json` parsing, env (KUBECONFIG/context/namespace), streamed output.
- UI: Releases view (columns Name/NS/Chart/Revision/Version/App version/Status/Updated), release detail (resources with health, values, manifest, notes, history), Charts catalog (repos, search, versions, README markdown), Install/Upgrade tab with values editor (E10 editor), Rollback, Uninstall; Repos management in settings.
**Out of scope:** OCI registry browsing (E29); native templating (never); Argo/Flux Helm sources (E25/E29).
**Crates:** oxikube_ports (HelmPort), oxikube_helm (native + cli), oxikube_app (HelmService), oxikube_helm_ui, oxikube_editor (values), oxikube_ui (markdown), oxikube_settings.
**Depends on:** E04, E07, E10, E12 (resource linking).
**Done when:** (1) releases list/history/values/manifest render with no helm binary; (2) install nginx from bitnami repo via UI on kind, upgrade with edited values, rollback, uninstall; (3) `helm` missing hides mutating UI and shows guidance; (4) 500-release cluster lists in <1 s; (5) Secret-driver and ConfigMap-driver fixtures both parse.
**Risks:** helm CLI version skew / output format → parse `-o json` only, pin minimum version, fixture tests per major; large values files → editor E10 handles; secrets in values → masking rules reuse E12-S11.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E16-S01 | `HelmPort` + domain types | S | Release, Revision, ChartRef, RepoEntry, Values; `HelmPort::{list_releases, history, values, manifest, notes, repos, search, show, template, install, upgrade, rollback, uninstall, capabilities()}`; fakes | oxikube_ports/helm.rs, oxikube_domain/helm.rs, oxikube_testkit | E02 |
| E16-S02 | Native release decoder (Secrets + ConfigMaps) | M | Decode `helm.sh/release.v1` payload; label-based listing per namespace; latest revision grouping; history; sofka/baeus-style, own implementation<br>Fixture tests (gzip payloads) incl. superseded/failed statuses | oxikube_helm/native.rs | S01, E04 |
| E16-S03 | Manifest → resources mapping | S | Parse rendered manifest docs → ResourceRefs → live objects + health from ResourceStore; missing objects flagged | oxikube_app/helm.rs | S02 |
| E16-S04 | `HelmCli` adapter | M | Detect binary (settings path override), version ≥3.10; run commands with cluster env; `-o json` parsers; streaming stdout for install/upgrade; timeouts; error mapping<br>Tests with recorded outputs | oxikube_helm/cli.rs | S01 |
| E16-S05 | Releases view + detail | M | Table + detail tabs (Resources w/ health, Values user/all toggle, Manifest, Notes, History); `:helm` jump; keymap context | oxikube_helm_ui/releases.rs | S02, S03, E07 |
| E16-S06 | Rollback + uninstall | S | History row → rollback to revision; uninstall with keep-history option<br>MutationGuard: blocked read-only; rollback confirm "medium", uninstall "high" (type name); Commands + tool stubs `helm_rollback`, `helm_uninstall` | oxikube_app/helm.rs, oxikube_helm_ui | S04, S05 |
| E16-S07 | Repos management | S | Settings page section: list/add/remove/update repos (CLI), stored in helm's own config; status refresh | oxikube_helm_ui/repos.rs, oxikube_settings_ui | S04 |
| E16-S08 | Charts catalog | M | Search across repos, chart cards (icon/name/desc/version/app version/repo), detail with README (markdown) + versions + default values | oxikube_helm_ui/charts.rs, oxikube_ui/markdown.rs | S04, S07 |
| E16-S09 | Install tab with values editor | M | Release name, namespace (create), version, values editor (E10 editor, YAML schema from `show values` comments) + dry-run preview (`template`/`--dry-run=server`) → install with streamed log<br>MutationGuard: confirm "low" after preview; Command `helm: install`; tool stub | oxikube_helm_ui/install.rs, oxikube_app/helm.rs | S04, S08, E10 |
| E16-S10 | Upgrade tab | M | Prefilled current values + chart version picker + diff of values vs current; `--reuse-values` toggle; dry-run preview; MutationGuard confirm "medium" | oxikube_helm_ui/upgrade.rs | S09 |
| E16-S11 | Capability gating + settings | S | `capabilities()` drives UI: native-only mode hides install/upgrade/rollback/uninstall/charts with guidance; settings: helm path, server-side apply flag, default repos | oxikube_helm_ui, oxikube_settings | S04 |
| E16-S12 | Helm integration tests | M | kind: native listing of a release created by helm CLI; install/upgrade/rollback/uninstall through adapter; no-binary mode | oxikube_testkit, integration.yml | S06, S10 |

### E17 — RBAC & access tooling  (Phase 2, Area: app+ui)
**Goal:** Lens-style RBAC views (ServiceAccounts, Roles, ClusterRoles, Bindings) with create/edit dialogs, plus k9s-style subject-centric views (users, groups, policy matrix, can-i), sidebar gating by `SelfSubjectRulesReview`, and ServiceAccount kubeconfig generation. Make "what can I / this subject do" answerable inside the app.
**In scope:**
- Kinds: ServiceAccount (secrets, image pull secrets, used-by), Role/ClusterRole (rule tables), RoleBinding/ClusterRoleBinding (subjects, roleRef), create/edit dialogs.
- AccessService: `SelfSubjectRulesReview` per namespace (cached, refreshed on connect/namespace change), `SelfSubjectAccessReview` can-i checks used to grey out actions; subject-centric index (users/groups from bindings → aggregated rules → per-verb matrix per API group).
- SA kubeconfig generation (token request via `TokenRequest` subresource, cluster CA, optional expiry).
- RBAC-aware sidebar and action gating hook (consumed by E12 KindSpec).
**Out of scope:** Argo CD RBAC (E25); audit log (E19); cluster user management (none).
**Crates:** oxikube_rbac_ui, oxikube_app (AccessService), oxikube_kube (reviews, token request), oxikube_ports (ResourcePort subresources), oxikube_resources_ui (gating hook), oxikube_domain (rbac models).
**Depends on:** E04, E07, E12-S01.
**Done when:** (1) sidebar hides sections the user cannot list, with a "restricted" indicator; (2) can-i matrix for a user/group/SA matches `kubectl auth can-i --list` on kind fixtures; (3) create RoleBinding dialog produces valid objects; (4) SA kubeconfig downloads and works with kubectl; (5) accessible-namespaces fallback works when cluster-wide namespace list is forbidden.
**Risks:** aggregated ClusterRoles and wildcard verbs → rule normaliser with tests; TokenRequest API availability (1.22+) → fallback to legacy token secrets with warning.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E17-S01 | RBAC domain models + rule normaliser | M | PolicyRule normalisation (wildcards, aggregated roles, resourceNames, nonResourceURLs); subject index (users/groups/SAs → bindings → rules)<br>Property tests vs `kubectl auth can-i --list` fixtures | oxikube_domain/rbac.rs, oxikube_app/access/index.rs | E02 |
| E17-S02 | `SelfSubjectRulesReview` + `SelfSubjectAccessReview` adapter | M | Create reviews via `Api::<SelfSubjectRulesReview>::create`; per-namespace cache with TTL; batch can-i<br>Accessible namespaces computed when cluster-wide list forbidden (Lens `accessibleNamespaces` setting as override) | oxikube_kube/access.rs, oxikube_app/access.rs | E04 |
| E17-S03 | Sidebar + action gating hook | S | KindSpec availability predicate uses AccessService; sections hidden/greyed with tooltip "no list permission"; mutating actions greyed when can-i false (soft gate before MutationGuard) | oxikube_resources_ui/sidebar.rs, oxikube_app/kinds.rs | S02, E12 |
| E17-S04 | ServiceAccount view + detail + used-by | S | Columns Name/NS/Secrets/Age; detail: secrets, imagePullSecrets, automount, bound roles (from index), used-by workloads | oxikube_rbac_ui/serviceaccount.rs | S01, E12 |
| E17-S05 | Role/ClusterRole views + create/edit dialog | M | Rule table (apiGroups/resources/verbs/resourceNames); aggregation labels shown; dialog builds rules with verb checkboxes and resource picker from discovery<br>MutationGuard: create/edit confirm "medium" (RBAC change); Commands + tool stubs | oxikube_rbac_ui/role.rs | S01, E10 |
| E17-S06 | RoleBinding/ClusterRoleBinding views + dialog | M | Columns Bindings/Role ref/Subjects; dialog: roleRef picker, subjects (User/Group/SA with ns); validation of kinds<br>MutationGuard confirm "medium"; Commands + tool stubs | oxikube_rbac_ui/binding.rs | S05 |
| E17-S07 | Subject-centric views: users, groups, policy matrix | M | `:usr`, `:grp`, `:rbac <subject>`: per-verb matrix (GET/LIST/WATCH/CREATE/PATCH/UPDATE/DELETE/DEL-LIST/EXTRAS) per API group; filter | oxikube_rbac_ui/subjects.rs | S01 |
| E17-S08 | Can-I tool (interactive + agent) | S | Dialog: verb/resource/ns/subject(impersonate) → result with reason; tool stub `can_i`; `@rbac/sa/ns/name` context provider | oxikube_rbac_ui/can_i.rs, oxikube_app/tools | S02 |
| E17-S09 | ServiceAccount kubeconfig generation | M | TokenRequest subresource (expiry setting) + cluster CA/server → kubeconfig YAML download/copy; legacy secret fallback with warning<br>MutationGuard: token creation confirm "low"; audit; Command `sa: generate kubeconfig` | oxikube_kube/token.rs, oxikube_app/access.rs | S04 |
| E17-S10 | RBAC integration tests | S | kind: restricted SA kubeconfig → app hides sections, matrix matches kubectl, binding creation works | oxikube_testkit, integration.yml | S03, S06, S09 |

### E18 — Cloud discovery  (Phase 2, Area: adapters+ui)
**Goal:** Discover EKS, GKE and AKS clusters by invoking the user's installed `aws`, `gcloud` and `az` CLIs (no SDK crates) and add them to the catalog with exec-auth kubeconfig entries, so cloud clusters appear next to kubeconfig ones with one click. Providers are hidden when their CLI is missing.
**In scope:**
- `CloudDiscoveryPort` with three CLI adapters: detection (binary + version + logged-in state), account/profile/project/subscription enumeration, region handling, cluster listing (`aws eks list-clusters/describe-cluster`, `gcloud container clusters list --format json`, `az aks list -o json`), kubeconfig entry generation (exec `aws eks get-token`; `gke-gcloud-auth-plugin`; `kubelogin`/`az aks get-credentials` style), written to an oxikube-managed kubeconfig file (never edits the user's `~/.kube/config`).
- Catalog UI: Cloud section with provider cards, refresh, filters (profile/region/project/subscription), "Add to catalog", status badges, errors with remediation (not logged in, plugin missing).
- Settings: enabled providers, CLI paths, profiles/regions/projects to scan, cache TTL.
**Out of scope:** SDK-based auth (never in v1); cloud consoles/costs; node group management.
**Crates:** oxikube_ports (CloudDiscoveryPort), oxikube_cloud (aws.rs, gcloud.rs, az.rs, process.rs), oxikube_app (DiscoveryService), oxikube_catalog_ui, oxikube_settings, oxikube_state_sqlite (cache), oxikube_testkit.
**Depends on:** E03, E05, E06.
**Done when:** (1) with `aws` configured, EKS clusters list per profile/region and connect via exec auth; (2) same for GKE and AKS on a machine with those CLIs; (3) missing CLI hides provider, missing auth plugin yields actionable error; (4) discovery runs off the UI thread with cancellation and caches results (TTL); (5) generated kubeconfig entries live in `~/.config/oxikube/kubeconfigs/<provider>.yaml` and are hot-reloaded by E06.
**Risks:** CLI output format drift → `--output json` only, versioned parsers with fixtures; slow CLIs (gcloud) → parallel per-region with timeouts, cached; MFA prompts (Lens #1077) → exec auth with interactive mode surfaced in terminal (E09) when needed.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E18-S01 | `CloudDiscoveryPort` + domain types + process runner | S | `Provider { id, detect(), scopes(), list_clusters(scope), kubeconfig_entry(cluster) }`; `DiscoveredCluster`; shared `CliRunner` (timeout, cancel, json parse, env/PATH from login shell); fakes | oxikube_ports/cloud.rs, oxikube_cloud/process.rs, oxikube_testkit | E02 |
| E18-S02 | AWS EKS adapter | M | Detect `aws` ≥2; profiles from config; regions (configured + setting); `eks list-clusters` + `describe-cluster` parallel; exec auth entry (`aws eks get-token --cluster-name … --profile …`, region); fixture tests | oxikube_cloud/aws.rs | S01 |
| E18-S03 | GCP GKE adapter | M | Detect `gcloud`, ADC/logged-in, projects list; `container clusters list --format json`; entry with `gke-gcloud-auth-plugin` exec (detect plugin, actionable error) | oxikube_cloud/gcloud.rs | S01 |
| E18-S04 | Azure AKS adapter | M | Detect `az`, subscriptions; `aks list -o json`; entry via `az aks get-credentials -f <tmp>` parsed or `kubelogin` exec; handles AAD clusters | oxikube_cloud/az.rs | S01 |
| E18-S05 | DiscoveryService + managed kubeconfig files | M | Runs providers off-thread with cancellation; merges into `~/.config/oxikube/kubeconfigs/<provider>.yaml` (atomic write, never touches user kubeconfig); registers file as a ClusterSource so E06 hot-reloads; cache in StatePort with TTL | oxikube_app/discovery.rs, oxikube_state_sqlite | S02–S04, E06 |
| E18-S06 | Catalog Cloud section UI | M | Provider cards (hidden when undetected), scope filters, cluster rows with status/version/region, "Add"/"Remove", refresh, progress + errors with remediation links; keymap context | oxikube_catalog_ui/cloud.rs | S05 |
| E18-S07 | Settings + schema | S | providers enabled, CLI paths, profiles/regions/projects/subscriptions lists, cache TTL, parallelism; hot reload | oxikube_settings | S05 |
| E18-S08 | Tools + tests | S | Tool stub `discover_clusters(provider)` read-only; recorded-CLI fixture tests for all three; manual smoke checklist doc for real accounts | oxikube_app/tools, oxikube_testkit | S06 |


### E19 — Safety & audit  (Phase 2, Area: app)
**Goal:** Every mutation in Oxikube passes through one MutationGuard that enforces per-cluster read-only mode, a confirmation policy scaled to blast radius, server-side dry-run previews, and an append-only local audit log. The same guard gates UI actions, command-bus commands, and (later) MCP tools, so no code path can bypass it.
**In scope:**
- `MutationGuard` service in `oxikube_app` with a `MutationIntent { cluster, namespace, gvk, name, verb, risk: Risk }` pipeline: read-only check → confirmation policy → optional dry-run → execute → audit.
- Per-cluster read-only toggle (persisted via StatePort), colour-coded cluster tabs/badges, "guard mode" status-bar indicator.
- Confirmation dialog tiers: none / simple confirm / type-the-name (Namespace, Node, PV, CRD, cluster-scoped delete, delete-collection, prune).
- Server dry-run diff before apply/edit/patch (reuses editor diff view from E10).
- Audit log table in SQLite (who = kube user/context, what, where, when, outcome, dry-run flag, initiated-by: ui|command|agent|plugin) + audit viewer tab + export.
- Secrets redaction in audit entries and logs.
**Out of scope:** editor diff rendering (E10); agent permission prompts (E27, which call MutationGuard with initiated-by=agent); Argo-specific prune/cascade confirmations (E25, implemented via the same guard tiers).
**Crates:** oxikube_domain (Risk, AuditRecord, MutationIntent), oxikube_ports (StatePort audit methods), oxikube_app (MutationGuard, AuditService), oxikube_state_sqlite, oxikube_workspace (dialogs, badges), oxikube_resources_ui (wiring).
**Depends on:** E02, E04, E05, E06, E07.
**Done when:** (1) with read-only on, every mutating action in UI and command bus returns `Error::ReadOnly` and the UI shows it, verified by an integration test over all registered commands; (2) deleting a Namespace requires typing its name; (3) edit/apply shows a server dry-run diff before commit; (4) every mutation appears in the audit viewer within 1s with correct fields; (5) `cargo xtask lint-deps` proves no UI crate calls ResourcePort mutating methods directly (grep-based lint rule).
**Risks:** Bypass via direct port calls — mitigate with the lint rule and by making mutating ResourcePort methods only reachable through `oxikube_app::mutation` (newtype wrapper). Dry-run unsupported on some aggregated APIs — fall back to "no preview available" banner, never block.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E19-S01 | Domain types: Risk, MutationIntent, AuditRecord, Initiator | S | Risk enum {Low, Medium, High, Irreversible} with doc'd mapping<br>MutationIntent + AuditRecord serde types<br>Unit tests for risk classification table (verb × scope × kind) | oxikube_domain::safety | E02 |
| E19-S02 | MutationGuard pipeline | M | `guard.execute(intent, op)` runs read-only → policy → dry-run → op → audit<br>Policy is a pure fn returning ConfirmTier<br>Fake ports tests cover every branch | oxikube_app::mutation | S01 |
| E19-S03 | Mutation newtype + lint rule | S | Mutating ResourcePort methods wrapped in `MutatingApi` only constructible in oxikube_app<br>xtask lint fails if ui/* references mutating methods<br>CI job added | oxikube_app, xtask | S02 |
| E19-S04 | Per-cluster read-only mode persistence + command | S | Toggle command `cluster::ToggleReadOnly` in CommandBus<br>Persisted per ClusterId via StatePort<br>Restored on launch | oxikube_app, oxikube_state_sqlite | S02, E06 |
| E19-S05 | Colour-coded cluster tabs, badges, status-bar guard indicator | M | Cluster colour setting (per-cluster settings layer)<br>Tab strip + status bar show colour and RO lock icon<br>Screenshot test via render_to_image | oxikube_workspace, oxikube_settings | S04, E05 |
| E19-S06 | Confirmation dialogs (simple + type-the-name) | M | Dialog component in oxikube_ui over gpui-component dialog<br>Type-the-name disables confirm until exact match<br>Keyboard: Enter confirms only when valid, Esc cancels | oxikube_ui, oxikube_workspace | S02 |
| E19-S07 | Dry-run preview integration | M | Guard requests `dry_run=All` via ResourcePort before apply/patch<br>Diff of live vs server-returned object shown using E10 diff view<br>Aggregated APIs without dry-run show banner | oxikube_app, oxikube_kube, oxikube_editor | S02, E10-S07 |
| E19-S08 | Audit log storage + migrations | S | SQLite table `audit` with indices (cluster, ts)<br>StatePort::append_audit/query_audit<br>Retention setting (days) + vacuum | oxikube_state_sqlite, oxikube_ports | S01 |
| E19-S09 | Audit viewer tab + export | M | Workspace item listing audit rows (virtualised table), filters by cluster/verb/initiator<br>Export JSONL/CSV via FsPort<br>Command `audit::Open` | oxikube_resources_ui (audit module) | S08, E07 |
| E19-S10 | Secret redaction policy | S | Redact Secret data + known token fields in audit, logs, crash reports<br>Shared `redact` module in oxikube_domain<br>Property tests | oxikube_domain::redact, oxikube_logging | S01 |
| E19-S11 | Integration test: read-only sweep over all commands | M | Test enumerates CommandBus registry, runs each mutating command with RO on against testkit fakes<br>Asserts ReadOnly error and zero port calls | oxikube_app tests, oxikube_testkit | S03, S04, E11 |

### E20 — Apply/kustomize, file transfer & cross-cluster copy/diff  (Phase 2, Area: app)
**Goal:** Cover the "bring manifests in and move things around" workflows Lens and k9s leave to kubectl: apply a local directory or kustomize build, copy files to/from pods (`kubectl cp` semantics over exec+tar), copy resources between clusters, and diff a resource across clusters/namespaces. All mutations go through MutationGuard (E19).
**In scope:**
- Apply directory (recursive, multi-document YAML/JSON, ordering by kind like kubectl), apply kustomize output via `kustomize build`/`kubectl kustomize` shell-out when installed.
- Apply result panel: per-object created/configured/unchanged/error, with server dry-run first.
- Pod file browser + transfer (list via `exec ls -la`/tar, download/upload via tar streams, progress, cancel), per-container.
- Copy resource to another cluster/namespace (strips server fields: status, uid, resourceVersion, managedFields, creationTimestamp, selfLink, generation; preserves labels/annotations minus kubectl last-applied) with conflict prompt.
- Cross-cluster/namespace resource diff view (normalised, server-managed fields ignored) reusing E10 diff.
**Out of scope:** Editor text editing (E10); Helm chart install (E16); confirmation tiers (E19).
**Crates:** oxikube_domain (ApplyPlan, Sanitize), oxikube_ports (FsPort, ExecPort tar streams), oxikube_app (ApplyService, TransferService, CopyService, DiffService), oxikube_kube, oxikube_resources_ui, oxikube_editor (diff view), oxikube_workspace.
**Depends on:** E04, E07, E09 (exec), E10, E19.
**Done when:** (1) applying a directory with 50 manifests to kind creates them in kubectl order and reports per-object status; (2) downloading a 100 MB file from a pod completes with progress and matches checksum; (3) copying a Deployment from cluster A to B produces an object kubectl considers valid without server fields; (4) diff of the same ConfigMap in two namespaces highlights only data differences.
**Risks:** tar availability in distroless images — mitigate with a clear error + fallback to `cat`/base64 for single files. Kustomize not installed — hidden action with install hint.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E20-S01 | Multi-document manifest loader | S | Parse YAML/JSON multi-doc from text/file/dir (recursive, .yaml/.yml/.json)<br>Preserves source path/line per doc for errors<br>Tests with CRLF, `---` edge cases, empty docs | oxikube_domain::manifests | E02 |
| E20-S02 | ApplyPlan ordering + sanitisation | S | Orders by kubectl kind precedence (Namespace, CRD, ServiceAccount, … last Pods/Jobs)<br>`sanitize_for_apply` strips server fields (table-driven)<br>Unit tests | oxikube_domain::apply | S01 |
| E20-S03 | ApplyService (SSA, dry-run first) | M | Applies each doc via ResourcePort::apply with field manager `oxikube`<br>Dry-run pass first, aggregated result per object<br>Goes through MutationGuard with Risk per kind | oxikube_app::apply | S02, E19-S02 |
| E20-S04 | Kustomize adapter | S | Detects `kustomize` or `kubectl kustomize`; runs build on a dir; feeds output to ApplyService<br>Hidden when neither binary exists<br>Stderr surfaced in result | oxikube_kube::kustomize | S03 |
| E20-S05 | Apply directory/kustomize UI + result panel | M | File/dir picker via FsPort; result table with status icons and error expansion<br>Re-run and "open in editor" per object<br>Command `apply::Directory` | oxikube_resources_ui::apply, oxikube_workspace | S03, S04 |
| E20-S06 | Exec tar streaming primitives | M | ExecPort helpers: `tar_download(pod, container, path) -> Stream<Bytes>`, `tar_upload`<br>Progress + cancel via CancellationToken<br>Integration test on kind busybox pod | oxikube_kube::transfer, oxikube_ports | E09-S03 |
| E20-S07 | Pod file browser item | L | Tree/list of remote dir (ls parsing with fallback to `find`), navigation, size/mode/mtime<br>Download/upload with progress toasts<br>Drag-drop upload from OS | oxikube_resources_ui::files | S06, E07 |
| E20-S08 | Copy resource to cluster/namespace | M | Command on any resource: pick target cluster/ns via picker<br>Sanitise + dry-run + conflict prompt (overwrite/skip/rename)<br>Audit records both clusters | oxikube_app::copy, oxikube_palette | S02, E19 |
| E20-S09 | Cross-cluster/namespace resource diff | M | Pick two ResourceRefs (same kind); normalise (sort keys, strip managed fields); side-by-side diff via E10 diff view<br>Ignore-list configurable in settings | oxikube_app::diff, oxikube_editor | E10-S07 |
| E20-S10 | Bulk apply/copy tests + fixtures | S | Fixture dir with 50 manifests incl. CRs<br>kind integration tests for apply order and copy<br>Testkit fake ExecPort with tar | oxikube_testkit, oxikube_app tests | S03, S08 |


### E21 — Settings & keymap UI  (Phase 3, Area: ui)
**Goal:** Give the settings core from E05 a Zed-like face: a searchable GUI settings page bound to typed setting field paths, a keymap editor with conflict detection, JSON schema generation so settings.json/keymap.json get validation in the manifest editor, and per-cluster override editing. Edits round-trip into the user's files preserving comments (vendored Zed settings_store/settings_json logic).
**In scope:**
- `cargo xtask gen-settings-schema` → `settings.schema.json`, `keymap.schema.json` (schemars) shipped in assets; editor (E10) associates them with the config files.
- Settings page (gpui-component `setting` + `form`): sections auto-derived from `SettingsSection` registrations; controls per type (bool, enum, number, string, list, colour, font); search; "reset to default"; file layer indicator (default/user/cluster); "open JSON" per section.
- Per-cluster override tab with layer precedence shown.
- Keymap editor: list of actions × bindings grouped by context, record-keystroke capture, conflict detection, unbind, per-OS defaults, vim keymap toggle; writes keymap.json preserving comments.
- Base keymap selector (default / k9s / vim) and "open keymap JSON".
**Out of scope:** SettingsStore core, layering, hot reload (E05); theme picker (E22); extension settings blocks (E23 contributes sections through the same registry).
**Crates:** oxikube_settings (schema gen, text edits), oxikube_keymap (edit ops, validation), oxikube_settings_ui, oxikube_ui, xtask.
**Depends on:** E05, E10, E11.
**Done when:** (1) every registered setting appears in the page with a working control and writes to settings.json without losing comments; (2) schema files validate the default settings and keymap in CI; (3) rebinding a key in the editor takes effect without restart and shows conflicts; (4) per-cluster override of namespace favourites edits `clusters.<id>` layer only.
**Risks:** Comment-preserving JSON edits are fiddly — vendor Zed's `settings_json::update_value_in_json_text` with GPL header and port its tests. Control coverage for nested structs — allow "edit in JSON" fallback for unsupported types.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E21-S01 | Vendor comment-preserving JSON edit (Zed settings_json) | M | `update_value_in_json_text(text, path, value)` ported with GPL header + attribution<br>Zed tests ported and passing<br>Used by SettingsStore::update_user | oxikube_settings::json_edit | E05 |
| E21-S02 | Settings schema generation (schemars) | S | `SettingsContent` derives JsonSchema; xtask writes settings.schema.json<br>CI diff check that schema is current<br>Dynamic enums injected (themes, keymaps) | oxikube_settings, xtask | E05 |
| E21-S03 | Keymap schema + validation | S | keymap.schema.json with action names enum from ActionRegistry<br>Validator reports unknown actions/contexts with line numbers<br>Shown as diagnostics when keymap.json is open in editor | oxikube_keymap, oxikube_editor | S02, E11 |
| E21-S04 | Settings page shell + search + navigation | M | Workspace item `settings::Open` (cmd-,); left nav of sections; fuzzy search over titles/descriptions<br>Layer badge per field<br>Screenshot test | oxikube_settings_ui | E05 |
| E21-S05 | Setting controls (bool/enum/number/string/list/colour/font) | M | Control factory keyed by schema type; debounced writes through S01<br>Reset-to-default per field<br>Invalid input shows inline error | oxikube_settings_ui::controls, oxikube_ui | S04, S01 |
| E21-S06 | Per-cluster override tab | S | Pick cluster → shows fields that support cluster scope; writes to `clusters.<id>` layer<br>Precedence explanation UI | oxikube_settings_ui, oxikube_settings | S05, E06 |
| E21-S07 | Keymap editor: list, search, context grouping | M | Virtualised table of action/context/binding/source<br>Filter by context, show unbound actions<br>Open-JSON link | oxikube_settings_ui::keymap | E11 |
| E21-S08 | Keymap editor: record keystroke, conflicts, write-back | M | Keystroke capture modal (modifier handling per OS)<br>Conflict detection across same context chain<br>Write via KeybindUpdateOperation preserving comments | oxikube_keymap::edit, oxikube_settings_ui | S07, S01 |
| E21-S09 | Base keymap + vim toggle | S | Setting `base_keymap: default\|k9s\|vim`; switching re-layers defaults live<br>Documented key table generated into docs/keymap.md by xtask | oxikube_keymap, xtask | S08 |
| E21-S10 | Settings UI tests | S | gpui tests: search finds field, toggle writes file, invalid number rejected<br>Uses init_with_dir (no watcher) | oxikube_settings_ui tests | S05 |

### E22 — Theming  (Phase 3, Area: platform)
**Goal:** Zed-compatible theming: import any Zed theme-family JSON (schema v0.2.0) unchanged, map it onto Oxikube tokens and gpui-component's ThemeConfig, add an `oxikube` block for Kubernetes status colours, support light/dark/system with a theme picker, icon themes, and hot reload of user theme directories.
**In scope:**
- `oxikube_theme`: ThemeFamily/Theme/ThemeTokens types; importer from Zed `style` map (with fallbacks for missing keys); `oxikube` extension block (status.{running,pending,failed,succeeded,unknown,terminating}, health.{healthy,progressing,degraded,suspended,missing}, syncStatus.*, diff.{added,removed,changed}); ThemeRegistry; SystemAppearance observer; `ActiveTheme` trait (`cx.theme().colors().…`).
- Bundled themes: One Dark/Light, Ayu, Gruvbox (Zed's, GPL-compatible) + a default "Oxikube" family.
- Theme picker (palette-driven) with live preview; settings `theme: {mode: system, light, dark}`.
- User themes dir `~/.config/oxikube/themes/` hot reload; themes from extensions (E23 registers via the same registry).
- Terminal ANSI palette from theme; syntax highlight mapping to editor highlighter.
**Out of scope:** extension packaging (E23); settings page controls (E21).
**Crates:** oxikube_theme, oxikube_ui (token application to gpui-component), oxikube_settings_ui (picker entry), oxikube_terminal, oxikube_editor.
**Depends on:** E05 (theme core stub), E11.
**Done when:** (1) all Zed themes in zed-industries/extensions sampled (≥20) import without error and render; (2) switching theme updates every open view without restart; (3) system appearance change flips light/dark automatically; (4) k8s status colours come from the theme block with sensible defaults when absent.
**Risks:** gpui-component theme fields diverge from Zed's keys — mapping table lives in one file with tests per key; unmapped keys logged once.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E22-S01 | Theme domain types + token struct | S | ThemeTokens struct (≈160 colours grouped), Appearance, ThemeMeta<br>Serde for Zed family JSON (lenient parser: comments, trailing commas)<br>schemars schema emitted | oxikube_theme::types | E05 |
| E22-S02 | Zed style importer with fallbacks | M | Map every Zed v0.2.0 key to tokens; fallback chain mirrors Zed's refine_theme<br>Table-driven tests; unmapped keys counted | oxikube_theme::import | S01 |
| E22-S03 | `oxikube` status colour block + defaults | S | Parse optional block; derive defaults from base palette when absent<br>Used by resources_ui status badges | oxikube_theme, oxikube_resources_ui | S02 |
| E22-S04 | ThemeRegistry + ActiveTheme + global | S | Registry insert/list/get/remove; `GlobalTheme`; `ActiveTheme` trait<br>Observe-able change event | oxikube_theme::registry | S02 |
| E22-S05 | Apply tokens to gpui-component ThemeConfig | M | Converter tokens→ThemeConfig incl. radius/fonts; applied on change<br>Verified visually via screenshot test of a component sheet | oxikube_ui::theme_bridge | S04 |
| E22-S06 | Bundled themes + assets | S | Bundle One/Ayu/Gruvbox + Oxikube default via rust-embed<br>License notices in THIRD_PARTY_NOTICES | oxikube_assets, oxikube_theme | S02 |
| E22-S07 | System appearance + mode setting | S | `theme` setting {mode, light, dark} with validation<br>Follows OS change events (gpui window appearance) | oxikube_theme, oxikube_settings | S04 |
| E22-S08 | Theme picker with live preview | M | Picker lists registry; arrow keys preview, Enter commits, Esc reverts<br>Command `theme::Select` | oxikube_settings_ui or oxikube_palette | S04, E11 |
| E22-S09 | User theme directory hot reload | S | Watch `~/.config/oxikube/themes`; load/replace/remove families<br>Invalid file → toast with error path | oxikube_theme::watch | S04 |
| E22-S10 | Terminal + editor palette mapping | S | ANSI 16 + cursor/selection from theme; syntax map to editor highlighter<br>Both update live | oxikube_terminal, oxikube_editor | S04, E09, E10 |
| E22-S11 | Icon theme support (file/kind icons) | M | IconTheme JSON (Zed icon_theme schema) → kind/status icons mapping<br>Default Lucide set; extension-provided icon themes later | oxikube_theme::icons, oxikube_assets | S04 |
| E22-S12 | Theme import conformance test | S | CI test imports ≥20 themes from a vendored sample set (licence-checked)<br>No panics, all required tokens present | oxikube_theme tests | S02, S06 |


### E23 — Extensions (WIT API, wasmtime host, install, UI, samples)  (Phase 3, Area: platform)
**Goal:** A Zed-identical, sandboxed extension system: extensions are WASM components built against `oxikube_extension_api` (WIT world `oxikube:extension/extension@since_v0.1.0`), declared by `extension.toml`, hosted by wasmtime 49 with epoch interruption and capability grants, installable from a local path or git URL, and able to contribute themes, icon themes, commands (palette entries + ':' aliases), and MCP servers. No UI hooks.
**In scope:**
- `oxikube_extension_api` crate (publishable): WIT files, `register_extension!` macro, host-call wrappers (settings get, http download, process exec under grants, key-value store, kube context info read-only), `cargo oxikube-ext new` template (xtask).
- Manifest `extension.toml` (id, name, version, schema_version, lib, themes, icon_themes, commands{id,title,description,scopes}, context_servers{id}, capabilities[]), parsing + validation.
- Host: shared Engine (component model, epoch interruption, compile cache ≤32 MB), per-extension Store + WASI ctx + working dir, `CapabilityGranter` (process:exec patterns, download_file URL patterns, kube:read contexts), run calls via gpui_tokio bridge, load/unload events via ExtensionHostProxy → theme registry, command registry, MCP registry.
- Install from path (dev extension) and from git URL (clone to extensions dir, build with `cargo build --target wasm32-wasip2` if source, or load prebuilt .wasm), enable/disable, uninstall, update check.
- Extensions UI: list installed, details, grants review/approve, errors; commands `extensions::Install`, `extensions::OpenDir`.
- Sample extensions in `extensions/`: theme-only, command (`hello`), MCP-server (spawns a stdio MCP server).
**Out of scope:** registry/marketplace (E29); ACP agent spawning (E27); declarative UI from plugins (explicitly not planned).
**Crates:** oxikube_extension_api, oxikube_extension_host, oxikube_extensions_ui, oxikube_theme (registration hook), oxikube_palette (command registration), oxikube_mcp (context server registration), xtask.
**Depends on:** E05, E11, E22, E26 (MCP registry for context_servers; can stub).
**Done when:** (1) sample extensions build to wasm32-wasip2 in CI and load on macOS+Linux; (2) a runaway extension (infinite loop) is interrupted within 1s without freezing UI; (3) an extension without `process:exec` grant cannot spawn a process (test); (4) installing from a git URL works offline-tolerant with a clear error; (5) WIT world versioned so a v0.1.0 extension loads under a v0.2.0 host.
**Risks:** wasmtime compile time and MSRV 1.96 — isolate in one crate, enable `cranelift` only, cache artifacts in CI. WIT churn — `since_vX` folders from day one; PENDING_CHANGES.md like Zed.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E23-S01 | WIT world v0.1.0 + extension_api crate | M | `wit/since_v0.1.0/{extension,common,http-client,process,platform,settings,kube-info}.wit`<br>`register_extension!`, Extension trait (init, commands, context-server-command)<br>Builds for wasm32-wasip2; doc'd | oxikube_extension_api | E05 |
| E23-S02 | Manifest schema + parser | S | extension.toml serde with validation errors incl. unknown fields<br>Capabilities enum (process:exec, download_file, kube:read)<br>Unit tests | oxikube_extension_host::manifest | — |
| E23-S03 | wasmtime engine + store + WASI ctx | M | Shared Engine (component model, epoch_interruption, async)<br>Per-extension Store with working dir and env scrubbing<br>Epoch ticker task; call timeout test | oxikube_extension_host::wasm_host | S01 |
| E23-S04 | Host imports + CapabilityGranter | M | Implement WIT imports: settings get, http download (allow-list), process exec (allow-list), kv store, kube contexts read<br>Denied calls return typed errors; tests per capability | oxikube_extension_host::capabilities | S03, S02 |
| E23-S05 | Version negotiation + compile cache | S | Parse wasm custom section for API version; load matching bindgen world<br>LFU cache capped 32 MB; cache hit test | oxikube_extension_host | S03 |
| E23-S06 | ExtensionHostProxy + registries wiring | M | Load/unload events; themes → ThemeRegistry; commands → CommandBus/palette with `ext:<id>:<cmd>` ids; context servers → MCP registry (stub ok)<br>Unload removes everything | oxikube_extension_host, oxikube_palette, oxikube_theme | S04, E22-S04, E11 |
| E23-S07 | Install from path (dev) + from git | M | Clone to `~/.config/oxikube/extensions/<id>`; detect prebuilt .wasm or build via cargo if toolchain present<br>Progress + error surfacing; enable/disable/uninstall | oxikube_extension_host::install | S06 |
| E23-S08 | Extensions manager UI | M | List with status, version, grants; details pane; approve/revoke grants dialog<br>Commands `extensions::Install/OpenDir`<br>Screenshot test | oxikube_extensions_ui | S07 |
| E23-S09 | Sample extensions + CI build | S | `extensions/{theme-sample,hello-command,mcp-echo}` building in CI<br>Loaded in an integration test on host | extensions/, .github | S06 |
| E23-S10 | xtask `ext new` template + docs | S | `cargo xtask ext new <id>` scaffolds manifest, lib.rs, build script<br>docs/extensions.md authoring guide | xtask, docs | S01 |
| E23-S11 | Runaway/abuse tests | S | Infinite loop interrupted; huge memory capped (store limits); fs access outside workdir denied | oxikube_extension_host tests | S03, S04 |

### E24 — Release engineering & updates  (Phase 3, Area: tooling)
**Goal:** Reproducible releases from tags: cargo-dist builds macOS (.dmg, universal or per-arch), Linux (.tar.gz, .deb, AppImage) artifacts with checksums; in-app auto-update (opt-out) against GitHub Releases with signature verification; opt-in anonymous crash reporting behind CrashReporterPort; notarization/signing turn on automatically when Developer ID secrets exist.
**In scope:**
- `dist-workspace.toml`/cargo-dist config, release.yml on tag push, SBOM/checksums, GitHub Release notes from conventional commits (git-cliff).
- macOS bundle: Info.plist, icons, entitlements; signing + notarization + stapling steps gated on secrets; Sparkle-free updater (download .dmg/.tar, verify minisign signature, replace bundle, relaunch).
- Linux: .desktop + icon (Wayland app_id), AppImage, .deb; updater replaces binary in user-writable installs only, otherwise shows instructions.
- UpdaterPort + oxikube_updater adapter: check on launch (debounced), channel setting (stable/preview), "release notes" dialog.
- CrashReporterPort + sentry adapter: off by default; consent dialog; redaction (E19-S10); local crash dumps always kept; "report issue" prefill GitHub issue.
- Version/build info command (`oxikube --version`, about dialog), release_channel crate-like module.
**Out of scope:** Windows packaging (E28); Homebrew tap/AUR (E29 backlog item).
**Crates:** xtask, bins/oxikube, oxikube_updater, oxikube_crash, oxikube_ports, oxikube_workspace (dialogs), oxikube_assets (icons), .github/workflows.
**Depends on:** E01, E05.
**Done when:** (1) pushing `v0.0.1-alpha.1` tag produces all artifacts + checksums + minisign sigs on a GitHub Release; (2) app on macOS detects a newer release, updates, relaunches; (3) with no Developer ID secrets, release still succeeds unsigned and logs why; with secrets, notarized .dmg passes `spctl --assess`; (4) crash reporting sends nothing until consent is given (network test with fake endpoint).
**Risks:** Notarization flakiness — retry + `notarytool --wait` timeout; unsigned Linux updater overwriting system installs — detect install path writability.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E24-S01 | cargo-dist setup + release workflow | M | dist config for macOS arm64/x86_64, Linux x86_64/arm64; release.yml on tags<br>Checksums + SBOM attached<br>Dry-run on PR via `dist plan` | dist-workspace.toml, .github | E01 |
| E24-S02 | macOS app bundle + icons | S | Info.plist (bundle id, min OS), icns, entitlements; `cargo xtask bundle`<br>Dock icon shows in bundled run | xtask, oxikube_assets | S01 |
| E24-S03 | Signing + notarization gated on secrets | M | Workflow step runs only if `APPLE_*` secrets set; notarytool submit/wait/staple<br>Unsigned path still publishes with notice | .github | S02 |
| E24-S04 | Linux packaging (.desktop, AppImage, .deb) | M | .desktop with app_id `dev.karan.oxikube`, icon sizes; AppImage via linuxdeploy; .deb via cargo-deb<br>Launches on Ubuntu 24.04 runner headless check | xtask, .github | S01 |
| E24-S05 | UpdaterPort + GitHub Releases adapter | M | Check latest by channel; compare semver; download asset + verify minisign signature<br>Settings: auto_update.enabled/channel | oxikube_ports, oxikube_updater | E05 |
| E24-S06 | In-app update flow (macOS/Linux) | M | Toast → release notes dialog → download progress → replace + relaunch (macOS bundle swap; Linux binary swap if writable else instructions)<br>Rollback on failed verify | oxikube_updater, oxikube_workspace | S05 |
| E24-S07 | Release notes generation (git-cliff) + conventional commits check | S | cliff.toml; notes attached to release; PR title lint workflow | .github, cliff.toml | S01 |
| E24-S08 | CrashReporterPort + sentry adapter (opt-in) | M | Port with consent state; sentry adapter sends only after consent; redaction applied<br>Local crash dump always written to logs dir | oxikube_ports, oxikube_crash, oxikube_logging | E19-S10 |
| E24-S09 | Consent dialog + "report issue" prefill | S | First-run consent off by default; settings toggle<br>Command `help::ReportIssue` opens GitHub new-issue URL with version/OS/log tail (redacted) | oxikube_workspace, oxikube_settings_ui | S08 |
| E24-S10 | Version/about + release channel module | S | `--version` prints semver+sha+channel; About dialog; channel compiled from env at build | bins/oxikube, oxikube_workspace | E05 |
| E24-S11 | Release smoke tests | S | Post-release workflow downloads artifacts, verifies checksums/sigs, runs `--version` on each OS runner | .github | S01, S04 |

### E25 — Integration framework + Argo CD + Rollouts  (Phase 4, Area: adapters + ui)
**Goal:** A generic `IntegrationPort` (detect → sidebar section, commands, MCP tools, context providers, settings) with Argo CD as the first full implementation: an `ArgoBackend` trait with three backends (CRD-direct via kube + kopium types; Server REST via hand-written reqwest client with SSE/NDJSON streams and cookie-authed terminal WS; Core helper spawning `argocd admin dashboard --port N`), capability-gated UI reaching Argo web-UI parity, plus Argo Rollouts (CRD-direct patches). Optional: nothing shows unless `applications.argoproj.io` exists or a server profile is configured.
**In scope:**
- Framework: `IntegrationPort { id, detect(cluster) -> Detection, contributions(): sidebar, commands, tools, context_providers, settings_section }`, `IntegrationRegistry` in app, detection on cluster connect + CRD watch, settings `integrations.<id>.enabled`.
- Argo domain: Application/ApplicationSet/AppProject/Rollout/AnalysisRun/Experiment view-models; capability enum (Diff, Manifests, RepoBrowse, ResourceTree, LuaActions, Accounts…); sync options model.
- Backend A CRD-direct: list/watch apps/appsets/projects; refresh/hard-refresh annotation; sync via `operation` merge patch (all options, selective resources); terminate; rollback from `status.history`; delete with cascade finalizers/non-cascade/deletion-approved annotation; spec edits; client-side resource tree rebuild from `status.resources` + ownerRefs; Argo health rules port for common kinds.
- Backend B Server REST: profiles (URL, TLS insecure/CA, headers, grpc-web root path), auth (token entry, import `~/.config/argocd/config`, SSO PKCE loopback via openidconnect, keychain storage, 401 re-login), endpoints for apps (list/get/stream, managed-resources, server-side-diff, manifests, resource get/patch/delete, actions list/run v2, events, logs stream, syncwindows, links, rollback, sync, terminate), appsets (list/get/tree/events/generate), projects (CRUD/detailed/roles/tokens/windows), repos/repocreds/clusters/certs/gpgkeys, account can-i, settings, version, terminal WS.
- Backend C Core helper: detect `argocd` binary, spawn `argocd admin dashboard --port <free> --kube-context <ctx>`, health-check, point B at loopback with no token, lifecycle tied to cluster session.
- UI: Argo sidebar section; Applications list (tiles/table, filters, sync/health badges, favourites); App detail (tree/list/network views, status panel, conditions, sync panel with all options, history & rollback, diff compact/inline/side-by-side, manifests, parameters/sources editing, events, logs, pod terminal, resource actions, delete dialog); ApplicationSets (list/detail/generated apps/generate preview); Projects (roles, windows, restrictions); Repos/Clusters/Creds/Certs/GPG settings pages; Accounts/tokens (B only); notifications config viewer; "Open in Argo UI"; capability banners naming which backend served the view and what's unavailable.
- Rollouts: list/detail (steps, canary/blue-green status, ReplicaSets→Pods, AnalysisRuns/Experiments), actions promote/promote-full/skip-step/skip-all/abort/retry/pause/resume/restart/set-image/undo via merge patches; terminate AnalysisRun/Experiment.
- MCP tools + @-mentions for Argo (registered through E26 APIs): argo.list_apps, argo.app_status, argo.diff, argo.sync (gated), argo.rollback (gated), rollouts.promote/abort (gated); `@app/<name>`.
**Out of scope:** Argo UI JS extensions (impossible natively); Argo Image Updater UI (backlog); Flux (E29); MCP server infrastructure (E26).
**Crates:** oxikube_domain::argo, oxikube_ports (IntegrationPort, ArgoBackend), oxikube_app (IntegrationRegistry, ArgoService), oxikube_argocd (adapters A/B/C, auth, rollouts), oxikube_argocd_ui, oxikube_keychain, oxikube_mcp (tool registration), oxikube_testkit (fixtures per Argo 3.3/3.4/3.5).
**Depends on:** E04, E06, E07, E09 (terminal element), E10 (diff view), E19, E26 (tool/context registration; can land with stubs).
**Done when:** (1) on a kind cluster with Argo CD core install, Applications list/sync/rollback/delete work with no Argo credentials; (2) with a server profile, diff/manifests/repo browse/projects admin work against Argo 3.5 and 3.4; (3) every Argo view declares required capabilities and renders a banner when the active backend lacks them; (4) Rollout promote/abort/retry verified on kind with argo-rollouts; (5) all Argo mutations go through MutationGuard and show prune/cascade warnings; (6) fixtures for 3.3/3.4/3.5 response shapes deserialise with `#[serde(default)]` tolerance.
**Risks:** Version skew of REST models — all fields optional, unknown-field tolerant, fixture tests per minor. Unvalidated CRD-direct writes — guard tiers + dry-run where possible + surface `requiresPruning`/`Delete=confirm`. Destination clusters unreachable in CRD mode — detect `spec.destination` ≠ control plane and label views. Unverified items → spikes S03–S05 before dependent stories.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E25-S01 | IntegrationPort + IntegrationRegistry + detection | M | Port trait with Detection {Available, Missing, Partial(reason)}; registry runs detect on connect + CRD add/remove<br>Sidebar/commands/tools/context/settings contributions typed<br>Settings `integrations.<id>.enabled` | oxikube_ports, oxikube_app::integrations, oxikube_workspace | E06 |
| E25-S02 | Argo domain types + kopium CRD types | M | kopium-generated Application/ApplicationSet/AppProject/Rollout/AnalysisRun/Experiment (v1alpha1) with `#[serde(default)]` everywhere<br>View-models + Capability enum + SyncOptions model<br>xtask `gen-argo-types` pinned to Argo v3.5.3 / Rollouts v1.10.0 CRDs | oxikube_domain::argo, oxikube_argocd::crd, xtask | — |
| E25-S03 | Spike: terminal WS protocol + Bearer on upgrade | S | Against a live Argo 3.5: confirm message ops (stdin/stdout/resize), cookie vs Bearer auth, ping/reconnect<br>Findings in docs/research/argocd-spikes.md | docs | — |
| E25-S04 | Spike: sync-window enforcement for CRD-written operations + SSE through proxies/ingress | S | Verify controller behaviour when `operation` is set during a deny window<br>SSE vs NDJSON through nginx ingress idle timeouts; choose default<br>Documented | docs | — |
| E25-S05 | Spike: `fields` filter param + `resourceHealthSource: appTree` handling | S | Confirm unary List `fields` param name; test CRD mode with appTree health source<br>Decide fallback UI | docs | — |
| E25-S06 | ArgoBackend trait + capability model | S | Trait with per-feature methods returning `Unsupported(Capability)`; `capabilities()`<br>Composite backend picks best available per call (A→C→B precedence configurable) | oxikube_ports::argo, oxikube_app::argo | S02 |
| E25-S07 | Backend A: list/watch apps/appsets/projects | M | Reflectors over CRDs incl. apps-in-any-namespace; status/sync/health/conditions/history exposed<br>Fixture tests | oxikube_argocd::crd_backend | S02, S06, E04 |
| E25-S08 | Backend A: refresh/sync/terminate/rollback via patches | M | Annotation refresh (normal/hard); `operation.sync` with all options + selective resources; terminate via operationState.phase; rollback from history (blocked when auto-sync on)<br>Through MutationGuard; kind integration test | oxikube_argocd::crd_backend | S07, E19 |
| E25-S09 | Backend A: delete (cascade/non-cascade/approved) + spec edits + auto-sync toggle | M | Finalizer handling; deletion-approved annotation; PUT/patch spec; syncPolicy toggle<br>Type-the-name confirm for cascade delete | oxikube_argocd::crd_backend | S08 |
| E25-S10 | Backend A: client-side resource tree + health port | L | Rebuild tree from status.resources + ownerReferences via live cluster; health rules for Deployment/StatefulSet/DaemonSet/Job/Pod/Service/Ingress/PVC/Rollout ported from Argo Lua semantics<br>Labelled "approximate" in UI | oxikube_argocd::tree, oxikube_app::argo | S07 |
| E25-S11 | Backend B: profiles + auth (token, CLI config import, keychain) | M | Profile model in settings (URL, insecure, CA, headers, grpc-web-root-path, port-forward flag)<br>Import `~/.config/argocd/config` contexts; tokens in keyring<br>401 → re-auth prompt | oxikube_argocd::auth, oxikube_keychain, oxikube_settings | S06 |
| E25-S12 | Backend B: SSO PKCE loopback login | M | openidconnect auth-code+PKCE, loopback listener on configurable port, reads /api/v1/settings oidcConfig/cliClientID, stores id+refresh tokens in keychain, refresh flow<br>Manual-token fallback | oxikube_argocd::sso | S11 |
| E25-S13 | Backend B: REST client core + streams | M | reqwest client with profile TLS/headers; NDJSON (default) + SSE decoders; resumable watch with resourceVersion; error type with Argo RBAC 403 mapping<br>Fixture tests 3.3/3.4/3.5 | oxikube_argocd::rest | S11 |
| E25-S14 | Backend B: application endpoints | L | list/get/stream, managed-resources, server-side-diff, manifests, resource get/patch/delete, actions list/run v2, events, logs stream, syncwindows, links, sync, rollback, terminate, delete, spec PUT/PATCH<br>Capability flags set | oxikube_argocd::rest::apps | S13 |
| E25-S15 | Backend B: appsets/projects/repos/clusters/creds/certs/gpg/accounts/settings/version | L | All CRUD endpoints per feature table; project roles/tokens/windows; account can-i pre-flight helper<br>Fixture tests | oxikube_argocd::rest::* | S13 |
| E25-S16 | Backend B: terminal WebSocket adapter | M | tokio-tungstenite with cookie auth (per S03), TerminalBackend impl feeding E09 terminal element; resize; reconnect | oxikube_argocd::terminal, oxikube_terminal | S03, S13, E09 |
| E25-S17 | Backend C: core helper process | M | Detect `argocd` binary + version skew; spawn `argocd admin dashboard --port <free> --kube-context`; readiness probe; stop on session close; logs captured<br>Backend B configured to loopback plaintext | oxikube_argocd::core_helper | S13 |
| E25-S18 | Backend B: auto port-forward to argocd-server (optional) | S | When profile has no URL and svc/argocd-server exists, port-forward via PortForwardPort and use insecure TLS to loopback<br>Toggle in profile | oxikube_argocd, oxikube_app::portforward | S11, E15 |
| E25-S19 | UI: Argo sidebar + Applications list | M | Tiles/table/summary views, filters (project/sync/health/cluster/labels/search), favourites, bulk sync/refresh<br>Capability banner | oxikube_argocd_ui::apps | S01, S07, E07 |
| E25-S20 | UI: Application detail (tree/list/network, status, conditions, history, events) | L | Tree with health/sync icons, node details + live manifest, hooks/waves markers; history with rollback action; events tab | oxikube_argocd_ui::app_detail | S19, S10, S14 |
| E25-S21 | UI: Sync panel + delete dialog + spec/parameters editing | M | All sync options incl. selective resources, retry, strategy, dry-run; delete cascade/propagation/non-cascade with confirm tiers; Helm/Kustomize/Directory/Plugin params + sources editor | oxikube_argocd_ui::sync | S08, S09, S14, E19 |
| E25-S22 | UI: Diff + manifests views | M | Live vs target diff (compact/inline/side-by-side, only-changed) via E10 diff; server-side diff toggle; manifests tab with revision picker<br>Unavailable in A/C-only → banner | oxikube_argocd_ui::diff, oxikube_editor | S14, E10 |
| E25-S23 | UI: logs + pod terminal + resource actions | M | Multi-pod logs via Argo endpoint or kube fallback; terminal tab via S16; Lua actions menu (B) with CRD-direct equivalents for restart/rollout actions | oxikube_argocd_ui, oxikube_logs_ui | S16, S14, E08 |
| E25-S24 | UI: ApplicationSets + Projects | M | AppSet list/detail/generated apps/generate preview (B); Project CRUD, roles/tokens (B), windows, source/dest restrictions | oxikube_argocd_ui::appsets, ::projects | S15, S07 |
| E25-S25 | UI: Repos/Clusters/Creds/Certs/GPG/Accounts/Notifications settings pages | M | CRUD forms with validate/test-connection (B); CRD-direct read of Secrets/ConfigMaps when B absent; notifications triggers/templates viewer | oxikube_argocd_ui::settings | S15 |
| E25-S26 | Rollouts: domain + CRD backend + actions | M | Reflectors for Rollout/AnalysisRun/Experiment; merge-patch actions (promote/full/skip/abort/retry/pause/resume/restart/set-image/undo/terminate) through MutationGuard<br>kind integration test with argo-rollouts | oxikube_argocd::rollouts | S02, E19 |
| E25-S27 | Rollouts UI | M | List + detail (strategy steps timeline, canary weight, RS→Pods, AnalysisRuns/Experiments), action buttons, status badges | oxikube_argocd_ui::rollouts | S26, E07 |
| E25-S28 | Argo MCP tools + @app mentions | S | Tools argo.list_apps/app_status/diff/sync/rollback, rollouts.promote/abort with permission gating; ContextProvider for `@app/<name>` (status, conditions, last op) | oxikube_argocd::tools, oxikube_mcp | E26-S03, S07 |
| E25-S29 | Argo settings section + "Open in Argo UI" + detection UX | S | Settings: profiles, backend precedence, enabled; sidebar hidden when undetected; open external URL from argocd-cm `url` | oxikube_argocd_ui, oxikube_settings | S01, S11 |
| E25-S30 | Fixture + conformance test suite | M | Recorded JSON fixtures for 3.3/3.4/3.5 (apps, tree, diff, streams); kind CI job installing argocd core + rollouts; tolerance tests | oxikube_testkit, .github | S07, S13 |


### E26 — Agent foundation (MCP tool server, context providers, command exposure)  (Phase 5, Area: app + adapters)
**Goal:** Make Oxikube's cluster capabilities available to any hosted agent through one `ToolRegistry` (typed tool definitions + invokers, permission-gated via MutationGuard) exposed as an MCP server (rmcp, stdio + streamable-HTTP) and one `ContextRegistry` that resolves `@`-mentions and "send to agent" payloads into text/JSON context blocks. Every feature epic registers its tools and providers here; the ACP panel (E27) only consumes.
**In scope:**
- `ToolPort`/`ToolRegistry`: `ToolDef { name, description, input_schema (schemars), risk, integration }`, `invoke(ctx, args) -> ToolResult`; namespacing `k8s.*`, `helm.*`, `argo.*`, `app.*`; per-tool permission policy (read tools auto, mutating tools require session/request_permission or UI prompt); tool call audit.
- Core tool set: k8s.list_contexts, k8s.list(gvk, ns, selector, limit), k8s.get, k8s.describe, k8s.events, k8s.logs(pod, container, tail, since, grep), k8s.top(nodes|pods), k8s.explain(gvk, field) (OpenAPI), k8s.diff_apply (dry-run); gated: k8s.apply, k8s.patch, k8s.delete, k8s.scale, k8s.restart, k8s.exec_once(cmd); app.* commands: app.open_view, app.select_resource, app.open_logs, app.search — bridging the CommandBus so agents can drive the UI; helm.* read tools.
- `ContextProviderPort`/`ContextRegistry`: mention grammar `@<kind>/<ns>/<name>`, `@logs/<pod>[/<container>]`, `@events/<ns>`, `@yaml/<ref>`, `@cluster`, `@namespace`, `@selection`; resolution to ContentBlocks with size budgets + truncation strategies; ambient context block (active cluster/ns/selection/read-only state); "send to agent" envelope from any view (table selection, log selection, detail panel, terminal output).
- `oxikube_mcp`: rmcp server exposing the registry; stdio transport (for ACP mcpServers passthrough) and streamable HTTP on loopback with per-session token; schema/`tools/list` reflects registry live; progress + cancellation.
- Settings: `agent.tools.allow/deny` patterns, per-cluster tool read-only enforcement, context size budgets.
**Out of scope:** ACP transport, agent panel UI, permission prompt UI (E27); Argo tools (E25 registers via S03 API).
**Crates:** oxikube_ports (ToolPort, ContextProviderPort), oxikube_app (ToolRegistry, ContextRegistry, tool impls over services), oxikube_mcp (rmcp adapter), oxikube_domain (ContextBlock, ToolDef, Mention), oxikube_resources_ui/logs_ui/terminal (send-to-agent hooks), oxikube_testkit.
**Depends on:** E04, E07, E08, E11 (CommandBus), E19.
**Done when:** (1) `oxikube` MCP server over stdio passes `tools/list` + `tools/call` for every core tool against kind (automated with an MCP test client); (2) a mutating tool call with read-only cluster returns a structured denial; (3) `@pod/default/nginx` resolves to a bounded context block with YAML + status + recent events; (4) "send to agent" from a log selection yields a ContentBlock with cluster/pod/container metadata; (5) tool invocations appear in the audit log with initiator=agent.
**Risks:** Context blocks blowing token budgets — per-provider budgets + head/tail truncation with markers. Tool schema drift — schemas generated from Rust types with tests that snapshot `tools/list`.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E26-S01 | Domain: ToolDef, ToolResult, ContentBlock, Mention grammar | S | Types with serde + schemars; mention parser with tests (edge cases: names with dots, cluster-scoped kinds)<br>Risk reuse from E19 | oxikube_domain::agent | E19-S01 |
| E26-S02 | ToolPort + ToolRegistry + permission policy | M | Register/list/invoke; policy: read auto, mutating → `PermissionRequest` callback; allow/deny settings patterns<br>Audit on invoke; fakes in testkit | oxikube_ports, oxikube_app::tools | S01, E19-S02 |
| E26-S03 | Integration tool registration API | S | `ToolRegistry::register_group(integration_id, tools)` + unregister on integration disable<br>Used by E25-S28 | oxikube_app::tools | S02, E25-S01 |
| E26-S04 | Core read tools (list/get/describe/events/logs/top/explain/contexts) | L | Implemented over ResourceStore/LogService/MetricsService/Describe/OpenAPI; output compact JSON + text summaries; limits + pagination cursors<br>Golden tests on fixtures | oxikube_app::tools::k8s | S02, E07, E08, E13 |
| E26-S05 | Gated mutating tools (apply/patch/delete/scale/restart/exec_once) | M | Each wraps MutationGuard with initiator=agent; dry-run result returned before permission; exec_once bounded time/output<br>Read-only denial test | oxikube_app::tools::k8s_mut | S04, E19 |
| E26-S06 | app.* UI-driving tools over CommandBus | M | open_view/select_resource/open_logs/search/run_command(name,args) mapped to Command enum with validation<br>Headless test using command bus fake UI | oxikube_app::tools::app | S02, E11 |
| E26-S07 | ContextProviderPort + ContextRegistry + budgets | M | Resolve mentions → ContentBlocks; per-provider byte budgets; truncation markers; ambient block builder<br>Property tests on truncation | oxikube_app::context | S01 |
| E26-S08 | Core context providers (resource/yaml/logs/events/cluster/namespace/selection) | M | Each provider formats deterministic text/JSON; secrets redacted; events sorted by last-seen | oxikube_app::context::providers | S07, E19-S10 |
| E26-S09 | "Send to agent" envelopes from views | M | Hooks in table selection, log selection, detail panel, terminal output (last N lines) → ContextRegistry::enqueue(block); indicator when no agent session open | oxikube_resources_ui, oxikube_logs_ui, oxikube_terminal | S07 |
| E26-S10 | oxikube_mcp server (rmcp) stdio + HTTP | M | rmcp ServerHandler reflecting registry; stdio transport binary mode (`oxikube --mcp-stdio` internal flag) and loopback streamable-HTTP with session token; progress/cancel | oxikube_mcp, bins/oxikube | S02 |
| E26-S11 | MCP conformance test client | S | Test harness using rmcp client: tools/list snapshot, call each read tool on kind, denial path | oxikube_mcp tests, oxikube_testkit | S10, S04 |
| E26-S12 | Agent settings section | S | `agent.tools.{allow,deny}`, `agent.context.budgets`, per-cluster `agent.read_only` override; schema + UI controls | oxikube_settings, oxikube_settings_ui | S02 |

### E27 — ACP client & agent panel  (Phase 5, Area: adapters + ui)
**Goal:** Host external coding/ops agents over ACP (agent-client-protocol 2.2, protocol v1) in a Zed-like agent panel: discover agents from the ACP registry + built-in presets (Claude Code, Codex, Antigravity, Gemini) + custom `agent_servers`, launch via npx/uvx/binary with checksum verification, run sessions with streaming updates, tool-call rendering, permission prompts routed through MutationGuard, virtualised `fs/*` so agent-proposed manifests open as editor diffs, terminal passthrough into the app terminal, elicitation forms, and @-mention/ambient/send-to-agent context from E26.
**In scope:**
- `AgentPort`/`oxikube_acp`: `Client.builder().on_receive_notification(SessionNotification).on_receive_request(RequestPermissionRequest | ReadTextFileRequest | WriteTextFileRequest | CreateTerminalRequest | TerminalOutputRequest | WaitForTerminalExitRequest | KillTerminalRequest | ReleaseTerminalRequest | elicitation)`; `connect_with(AcpAgent)`; initialize with ClientCapabilities (fs read/write, terminal, elicitation form/url); session/new with cwd + MCP servers (oxikube_mcp stdio config), load/list/resume/close/delete when advertised; session/set_mode, set_config_option; prompt with ContentBlocks; cancel; auth_methods incl. terminal-auth flow.
- Registry: fetch `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`, schema-validate, cache with TTL, offline fallback to built-in presets; launchers: npx (node check), uvx, binary (per-platform download, sha256/minisign verify, cache dir, exec bit); version pinning + update notice; licence notice for proprietary adapters (fetched, never bundled).
- Thread model (vendored design of Zed acp_thread): AgentThread, entries (user/assistant chunks, thoughts, tool calls with status/content: text, diff, terminal), plan, permission requests, elicitations, modes/config options, available slash commands, usage/metadata; persistence of thread transcripts in SQLite.
- Agent panel UI (dock panel + movable item): agent picker, session list, message editor with @-mention completion (ContextRegistry), slash commands, streaming markdown, collapsible tool calls, diff cards → "open in editor" / "apply" (gated), terminal cards embedded via oxikube_terminal display-only backend, permission prompt cards (allow once/always/deny) mapped to MutationGuard tiers, elicitation forms, plan view, mode/config selectors, cancel/retry, cost/usage if provided.
- fs virtualisation: `fs/read_text_file` serves cluster resources via `oxikube://<cluster>/<gvk>/<ns>/<name>.yaml` virtual paths and real files under an allowed cwd; `fs/write_text_file` on virtual paths → "proposed manifest" → editor diff + apply gated; real paths under cwd allowed with permission.
- Terminal passthrough: terminal/create runs in app terminal (local PTY backend) with env (KUBECONFIG/context), output capture, exit wait, kill/release; visible as a tab.
- Presets: claude-acp (npx @agentclientprotocol/claude-agent-acp), codex-acp, antigravity-acp (binary), gemini (npx @google/gemini-cli --acp), custom `agent_servers { name: {command, args, env} }`; per-agent env/proxy passthrough; auth status UI.
**Out of scope:** tool implementations and context providers (E26); Oxikube's own LLM/chat (none planned); MCP server internals (E26).
**Crates:** oxikube_ports (AgentPort), oxikube_domain::agent (thread model), oxikube_app (AgentSessionManager), oxikube_acp (SDK client, registry, launchers), oxikube_agent_ui, oxikube_terminal, oxikube_editor, oxikube_state_sqlite (threads), oxikube_keychain (agent tokens if any), oxikube_settings.
**Depends on:** E05, E09, E10, E11, E19, E26.
**Done when:** (1) Claude Code and Codex sessions run end-to-end: prompt with `@pod/...` mention → tool call via oxikube MCP → permission prompt → result rendered; (2) Antigravity binary adapter downloads, verifies checksum, launches on macOS/Linux; (3) agent-written YAML never touches disk: appears as editor diff and applies only after confirmation; (4) terminal/create runs visibly in the app terminal and the agent receives output/exit code; (5) read-only cluster blocks mutating tool calls even if the agent requests them; (6) thread survives app restart (transcript restored); (7) protocol conformance tests with `agent-client-protocol-test` fake agent pass.
**Risks:** ACP SDK churn (1.0→2.2 in 3 months) — pin `=2.2.0`, isolate behind AgentPort, adapter crate only. npx/node absent — detect and show install guidance; prefer binary distributions when registry offers them. Proprietary adapter terms — runtime fetch only, show licence/notice on first launch.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E27-S01 | Thread domain model (vendored acp_thread design) | M | AgentThread/Entry/ToolCall{status,content: text\|diff\|terminal}/Plan/PermissionRequest/Elicitation/Mode/ConfigOption types; reducers applying SessionUpdate variants; unit tests from recorded update streams | oxikube_domain::agent::thread | E26-S01 |
| E27-S02 | AgentPort + AgentSessionManager | M | Port: connect(agent_spec) → session handle; prompt/cancel/set_mode/set_config/close; event stream of thread updates<br>Manager tracks sessions per cluster context; fakes in testkit | oxikube_ports, oxikube_app::agent | S01 |
| E27-S03 | oxikube_acp: SDK client + stdio spawn + initialize | M | `agent-client-protocol =2.2.0` Client builder with notification/request handlers; AcpAgent spawn with env/proxy; InitializeRequest v1 + ClientCapabilities; AgentCapabilities stored | oxikube_acp::client | S02 |
| E27-S04 | Session lifecycle + prompt streaming + cancel | M | session/new with cwd + mcpServers (oxikube_mcp stdio); load/list/resume/close/delete when supported; PromptRequest with ContentBlocks; session/cancel + $/cancel_request; updates → S01 reducers | oxikube_acp::session | S03, E26-S10 |
| E27-S05 | request_permission bridge → MutationGuard + UI | M | Map permission options to tiers; "allow always" scoped per session+tool; denial when cluster read-only without prompting; audit | oxikube_acp, oxikube_app::agent, oxikube_agent_ui | S04, E19 |
| E27-S06 | fs/read + fs/write virtualisation | M | Virtual `oxikube://` paths resolve via ResourceStore; real paths restricted to session cwd allow-list; write on virtual → ProposedManifest event; write on real path requires permission | oxikube_acp::fs, oxikube_app::agent | S04, E26-S08 |
| E27-S07 | Proposed manifest → editor diff → gated apply | M | Opens E10 editor in diff mode (live vs proposed); Apply runs dry-run + MutationGuard; result reported back to agent as tool content | oxikube_agent_ui, oxikube_editor | S06, E10 |
| E27-S08 | terminal/* passthrough | M | create/output/wait_for_exit/kill/release over oxikube_terminal local PTY backend with cluster env; output ring buffer with byte limit; tab shown in dock | oxikube_acp::terminal, oxikube_terminal | S04, E09 |
| E27-S09 | Elicitation (form + url) handling | S | Render elicitation/create as form (schema-driven) or open URL; complete/cancel responses | oxikube_acp, oxikube_agent_ui | S04 |
| E27-S10 | Registry fetch + schema validation + cache | M | Fetch registry.json, validate against vendored schema, cache with TTL + offline fallback to presets; entries → AgentSpec {dist: npx\|uvx\|binary} | oxikube_acp::registry | S03 |
| E27-S11 | Launchers: npx/uvx/binary with checksum + cache | M | Node/uv detection with guidance; binary per-platform download to cache dir, sha256/minisign verify, exec bit, version dir; licence notice dialog for proprietary adapters | oxikube_acp::launch | S10 |
| E27-S12 | Presets + custom agent_servers settings | S | Built-in specs for claude-acp, codex-acp, antigravity-acp, gemini; settings `agent_servers` schema; env/proxy passthrough | oxikube_acp::presets, oxikube_settings | S11 |
| E27-S13 | Auth flows (auth_methods, terminal-auth) | M | Handle authenticate; terminal-based login spawns in app terminal; status shown; retry | oxikube_acp::auth, oxikube_agent_ui | S08, S03 |
| E27-S14 | Agent panel shell: picker, session list, layout | M | Dock panel + detachable item; agent/session pickers; new/close/rename; keyboard focus contexts | oxikube_agent_ui::panel | E05, S02 |
| E27-S15 | Message editor with @-mention + slash commands | M | Mention completion from ContextRegistry; slash commands from available_commands; attachments from send-to-agent queue; submit/cancel keys | oxikube_agent_ui::editor, oxikube_ui | S14, E26-S07 |
| E27-S16 | Thread view: streaming markdown, thoughts, tool-call cards | L | Virtualised entries; markdown via gpui-component; collapsible tool calls with status; diff cards (S07) and terminal cards (display-only backend); plan view | oxikube_agent_ui::thread | S14, S01 |
| E27-S17 | Permission + elicitation cards, mode/config selectors | M | Inline cards with allow-once/always/deny; forms; mode and config option dropdowns from session state | oxikube_agent_ui | S05, S09 |
| E27-S18 | Thread persistence + restore | M | Transcripts + metadata in SQLite via StatePort; restore on launch; session/load when agent supports, else read-only history | oxikube_state_sqlite, oxikube_app::agent | S04 |
| E27-S19 | Send-to-agent + ambient context wiring | S | Queue from E26-S09 attaches to next prompt; ambient block prepended per settings; UI chips show attachments | oxikube_agent_ui, oxikube_app::context | S15, E26-S09 |
| E27-S20 | Conformance + e2e tests | M | Fake agent via `agent-client-protocol-test`; e2e on kind with Claude/Codex adapters (manual-gated job); screenshot tests of panel | oxikube_acp tests, oxikube_agent_ui tests, .github | S04, S16 |

### E28 — Windows support  (Phase 6, Area: platform + tooling)
**Goal:** Bring Oxikube to Windows on GPUI's native DirectX 11/DirectWrite backend: build in CI with MSVC, fix platform-specific gaps (PTY via ConPTY, paths, keychain via Windows Credential Manager, HiDPI, IME, window chrome), package a signed-when-possible installer/zip, and run an interactive smoke-test checklist on a real machine.
**In scope:** MSVC toolchain + Windows SDK in CI (CMake only if wasmtime needs it); `winresource` icon/version; windows subsystem for release; ConPTY backend for oxikube_terminal (portable-pty or direct); `keyring` Windows backend; path/config dir conventions (%APPDATA%\Oxikube); file permission hardening equivalent (ACL) for config/secrets; kubeconfig/exec plugin quirks (`.exe`, PowerShell shells); updater for zip/MSIX; packaging (zip + optional MSI/MSIX via cargo-dist/wix), Authenticode signing gated on secrets; interactive smoke checklist (title bar controls, HiDPI scaling, IME, drag-drop, notifications, fonts).
**Out of scope:** ARM64 Windows (backlog), Microsoft Store.
**Crates:** bins/oxikube, oxikube_terminal, oxikube_keychain, oxikube_updater, oxikube_logging, xtask, .github.
**Depends on:** E05, E09, E24.
**Done when:** (1) CI builds + tests on windows-latest; (2) app launches, connects to a cluster, opens a terminal and logs on a physical Windows 11 machine (checklist signed off); (3) release produces a Windows zip (+MSI when wix available) with `--version` smoke; (4) no secrets written outside Credential Manager.
**Risks:** Nobody in the GPUI K8s ecosystem has run interactively on Windows — budget a spike first; ConPTY resize/encoding issues — use UTF-8 code page and test with PowerShell + cmd.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E28-S01 | Spike: build + launch on Windows | M | Document toolchain (MSVC, SDK, Spectre libs), fix compile errors, capture first-launch issues list | docs, bins | E05 |
| E28-S02 | CI job windows-latest | S | fmt/clippy/test on Windows; cache; wasmtime CMake if needed | .github | S01 |
| E28-S03 | Paths, config dir, ACL hardening | S | %APPDATA%\Oxikube; logs dir; ACL restrict on secrets/config | oxikube_logging, oxikube_settings, oxikube_state_sqlite | S01 |
| E28-S04 | ConPTY terminal backend | M | Local shells (pwsh/cmd/wsl) via ConPTY; resize; UTF-8; env injection | oxikube_terminal | S01, E09 |
| E28-S05 | Keychain (Credential Manager) + kubeconfig exec quirks | S | keyring windows backend tested; exec plugins with .exe resolution; proxy env | oxikube_keychain, oxikube_kube | S01 |
| E28-S06 | Window chrome, HiDPI, IME, notifications fixes | M | Title bar buttons, DPI change handling, IME composition in editor/palette, toast notifications | oxikube_workspace, oxikube_ui | S01 |
| E28-S07 | Packaging: zip + MSI (wix) + Authenticode gated | M | cargo-dist Windows targets; winresource; signing step when secrets exist; updater zip replace | xtask, .github, oxikube_updater | S02, E24 |
| E28-S08 | Interactive smoke checklist + sign-off | S | docs/windows-smoke.md executed on real hardware; issues filed | docs | S04–S07 |

### E29 — Backlog (post-v1 candidates)  (Phase 6, Area: mixed)
**Goal:** Park well-scoped follow-ups so they are visible on the roadmap without blocking v1. Each story is a seed to be expanded into its own epic when picked up.
**In scope (seeds):** Flux integration via IntegrationPort; curated `oxikube-extensions` registry + in-app browser; cross-cluster aggregated views (all pods across clusters, fleet health); accessibility pass (AccessKit roles via gpui-component, screen-reader audit, keyboard-only audit); image vulnerability scanning (syft/grype or trivy CLI adapter); Homebrew tap + AUR + winget; Windows ARM64; Argo Image Updater UI; i18n scaffolding (rust-i18n); saved searches/advanced filters; k9s plugin YAML import (hotkeys/commands); Prometheus/VictoriaMetrics/Mimir explorer; cluster cost view (opencost); OIDC device-flow helper for kubeconfig.
**Out of scope:** anything already in E01–E28.
**Crates:** varies. **Depends on:** v1 complete. **Done when:** each seed is either promoted to an epic or closed with a decision note. **Risks:** scope creep — seeds require an ADR before promotion.

**Stories:**
| ID | Title | Size | Acceptance criteria | Crates/modules | Depends |
|---|---|---|---|---|---|
| E29-S01 | Flux integration (seed) | L | Detect Flux CRDs; Kustomization/HelmRelease/GitRepository views; suspend/resume/reconcile via annotations; MCP tools | oxikube_flux, oxikube_flux_ui | E25-S01 |
| E29-S02 | Extension registry + in-app browser (seed) | L | `oxikube-extensions` repo with index + CI builds; browse/install/update in UI | oxikube_extension_host, oxikube_extensions_ui | E23 |
| E29-S03 | Cross-cluster aggregated views (seed) | L | Fleet view across warm clusters; per-cluster colour; palette search across clusters | oxikube_app, oxikube_resources_ui | E06, E07 |
| E29-S04 | Accessibility audit + fixes (seed) | M | AccessKit roles on custom elements; VoiceOver/NVDA checklist; focus order | oxikube_ui, oxikube_terminal, oxikube_editor | E05 |
| E29-S05 | Image vulnerability scanning (seed) | M | trivy/grype CLI adapter; per-image findings view; severity badges on pods | oxikube_scan, oxikube_resources_ui | E12 |
| E29-S06 | Package managers: Homebrew tap, AUR, winget (seed) | S | Formula/cask, PKGBUILD, winget manifest automated from release | .github | E24, E28 |
| E29-S07 | k9s config import (aliases/hotkeys/plugins) (seed) | M | Import aliases.yaml/hotkeys.yaml; plugins as commands running in terminal with env vars | oxikube_keymap, oxikube_palette | E11 |
| E29-S08 | Saved searches + advanced filter language (seed) | M | Persisted filters per cluster; label/field/regex grammar; palette entries | oxikube_app::search, oxikube_resources_ui | E07 |
| E29-S09 | i18n scaffolding (seed) | S | rust-i18n keys for UI strings; language setting; en only initially | oxikube_ui, oxikube_settings | E21 |
| E29-S10 | Metrics explorer (PromQL) + cost view (seed) | M | Ad-hoc PromQL panel with charts; opencost adapter | oxikube_prometheus, oxikube_overview_ui | E13 |


