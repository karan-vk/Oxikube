# Oxikube — instructions for coding agents (Codex, Gemini, others)

Oxikube is a native Kubernetes desktop client in Rust + GPUI (Zed's UI framework), built
by several agent teams in parallel from a GitHub Project of epics and stories. The full
working agreement lives in `.claude/skills/oxikube-contributor/SKILL.md` and its
`references/` directory; read them. The rules below are repeated here in full because
your harness may not load that skill automatically.

## Repository shape
```
crates/domain/oxikube_domain     pure types, no I/O, no gpui, no kube, no k8s-openapi
crates/ports/oxikube_ports       async_trait port traits (depends on domain only)
crates/app/oxikube_app           services + use-cases (domain + ports only; NO gpui, NO kube)
crates/adapters/oxikube_*        port implementations (kube-rs, argocd, helm, sqlite, keychain, acp, mcp...)
crates/platform/oxikube_*        settings, keymap, theme, runtime bridge, extension host (domain + ports, gpui ok)
crates/ui/oxikube_*              GPUI views; gpui-component ONLY via oxikube_ui
crates/testing/oxikube_testkit   fakes for every port, fixtures, gpui test helpers
bins/oxikube                     wires everything; init order
xtask                            lint-deps, check-gpui-pin, kind-up/down, load-pods, screenshots
docs/                            ARCHITECTURE.md, CONTEXT.md, adr/, research/, PLAN.md
```
Dependency direction: `domain <- ports <- app <- ui/bins`; adapters implement ports and
never depend on app or ui; platform depends on domain/ports. `cargo xtask lint-deps`
fails CI on any other edge and on banned crates per layer.

## Rules reviewers enforce
1. Layer rules above are absolute. Move code rather than adding an edge.
2. No `gpui`, `kube`, or `gpui_component` outside their allowed layers.
3. Every mutation goes through `MutationGuard` in `oxikube_app`: read-only check,
   confirmation tier (none / simple / type-the-name), server dry-run where available,
   audit record. UI code never calls a mutating port method directly.
4. Every user-facing action is a `Command` in the `CommandBus` and registers an MCP
   tool definition stub, so palette, keymap, menus and hosted agents share one path.
5. No secrets on disk. Tokens and decoded Secret data go to the keychain port or stay in
   memory; logs and audit entries are redacted; terminal scrollback is never persisted.
6. GPUI dependencies are exact pins (`gpui-pre*`, `gpui-component`, `gpui-base`,
   `gpui-kit-assets`) bumped together in one PR; `cargo xtask check-gpui-pin` verifies.
   Never add a git dependency on zed-industries/zed.
7. GPUI `Task`s must not drop themselves (use a flag + `.detach()`); Kubernetes work runs
   through `oxikube_runtime::spawn_kube` (tokio, abort-on-drop); never block the UI thread;
   coalesce `cx.notify()` for streams.
8. Copied code keeps its licence: Zed code gets the GPL-3.0-or-later header from
   `references/zed-vendoring.md` and an entry in `THIRD_PARTY_NOTICES.md`; kdash code keeps
   its MIT notice. Prefer copying Zed's design over its code.
9. One story = one branch (`story/E07-S03-short-slug`) = one PR. Conventional commits
   (`type(scope): summary`, no co-author trailers). Every PR is reviewed with Claude
   Code's `/code-review high` on Sonnet 5.5 (`claude-sonnet-5-5`): the author runs it
   before opening the PR and resolves every confirmed finding; the reviewer runs
   `/code-review high <PR#> --comment` on the PR. If your harness cannot run that skill,
   leave the PR in draft and request the review from a Claude Code teammate; CI also runs
   it via `.github/workflows/pr-review.yml`. CI green + one reviewer approval,
   squash merge. No changes outside the story's scope; file follow-up issues.
10. Tests ship with the story: unit tests with `oxikube_testkit` fakes for app/domain,
    kind integration tests (`--features integration`) for adapters, `#[gpui::test]` for UI
    (screenshots where the story says so). Tests must be deterministic: no OS threads,
    no sleeps.

## Workflow
1. Pick a story whose dependencies are Done; read its epic and acceptance criteria.
2. Set Project Status to `In Progress`; branch from `main`.
3. Implement + tests; run the gate below; run `/code-review high` (Sonnet) and fix
   findings; open a PR with the template filled (incl. the Code review section); set
   Status `In Review`.
4. Address review; squash merge; Status `Done`.

## Local gate
```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask lint-deps
cargo xtask check-gpui-pin
cargo deny check
cargo xtask kind-up && cargo test -p oxikube_kube --features integration   # adapters
```

## Key technical choices (do not relitigate without an ADR)
- Rust edition 2024; `rust-toolchain.toml` pins the toolchain; `[workspace.dependencies]`.
- kube 4.x with explicit features and the `ring` TLS provider; k8s-openapi `latest`;
  `jiff` for time; `serde-saphyr` for YAML.
- Domain `Resource` = metadata + GVK + raw JSON; typed view-models built from JSON.
- Table data: typed/metadata reflectors + own columns for core kinds; server Table API
  (hand-rolled Accept header) for CRDs.
- Settings/keymap/theme follow Zed's layered JSON designs; themes import Zed theme JSON.
- Extensions: wasmtime component model + WIT, `extension.toml`, capability grants.
- Agents: `agent-client-protocol =2.2.0` client; Oxikube exposes an MCP server of
  cluster tools; all agent mutations go through `MutationGuard`.
- Argo CD: three backends (CRD-direct, server REST, `argocd` core helper) behind one trait.
