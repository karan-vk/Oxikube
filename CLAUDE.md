# Oxikube — agent entry point

Oxikube is a native Kubernetes desktop client in Rust + GPUI, built by agent teams from a
GitHub Project of epics and stories. Before doing anything in this repo, load the contributor
skill: `.claude/skills/oxikube-contributor/SKILL.md` (its `references/` hold the detail).
It is the working agreement; reviewers enforce it.

## Eleven non-negotiables (short form)
1. Hexagonal layers: `domain <- ports <- app <- ui/bins`; adapters implement ports; platform -> domain/ports. `cargo xtask lint-deps` must pass.
2. No `gpui` / `kube` / `gpui_component` outside their allowed layers; views use gpui-component only via `oxikube_ui`.
3. Every mutation goes through `MutationGuard` (read-only, confirmation tier, dry-run, audit); UI never calls mutating ports directly.
4. Every user action is a `Command` in the `CommandBus` and registers an MCP tool stub.
5. No secrets on disk: keychain or memory only; logs/audit redacted; no scrollback persistence.
6. GPUI pins are exact (`gpui-pre*` + `gpui-component` family) and bumped together; never a Zed git dependency.
7. No self-dropping GPUI `Task`s; Kubernetes work via `oxikube_runtime::spawn_kube` (abort-on-drop); nothing blocking on the UI thread.
8. Vendored code carries its licence header (GPL for Zed, MIT for kdash) and a `THIRD_PARTY_NOTICES.md` entry.
9. Story = branch (`story/E07-S03-slug`) = PR; conventional commits; `/code-review medium` on Sonnet (`/model sonnet`) before opening the PR and again by the reviewer on the PR, every confirmed finding resolved; CI green + reviewer-agent approval; squash merge; no out-of-scope changes.
10. Tests ship with the story: testkit fakes for app/domain, kind integration for adapters, `#[gpui::test]` (+ screenshots) for UI.
11. Smooth as Zed: budgets in `docs/PERFORMANCE.md` (ADR 0013) — p95 frame ≤ 8 ms under 10k-pod churn, input ≤ 1 frame, cold start ≤ 400 ms, nothing blocking on the UI thread, virtualised lists, coalesced notify; hot-path PRs include `--perf` numbers.

## Commands
```
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask lint-deps
cargo xtask check-gpui-pin
cargo deny check
cargo xtask setup   # install pre-commit (fmt, lint-deps, gpui-pin) + pre-push (clippy, deny) hooks
cargo xtask kind-up && cargo test -p oxikube_kube --features integration
cargo x <subcommand>                         # short alias for cargo xtask <subcommand>
cargo it                                     # run the integration test suites (needs a kind cluster)
```

## Docs
- `docs/ARCHITECTURE.md` — canonical crate map and layer rules
- `docs/CONTEXT.md` — domain glossary (use its terms in type names)
- `docs/adr/` — decisions; add one when you change one
- `docs/research/` — feature inventories and ecosystem research behind the plan
- `docs/PLAN.md, docs/DEPENDENCIES.md (dependency matrix, waves, critical path)` — full epic and story catalogue

## Conventions
- Rust edition 2024, toolchain pinned in `rust-toolchain.toml`; `[workspace.dependencies]` only.
- Commits: `type(scope): summary`; no co-author trailers.
- Project board: claim by setting Status `In Progress`; `In Review` when the PR opens.
- Code review: every PR gets `/code-review medium` twice (author pre-PR, reviewer on PR), model Sonnet 5.5; see `.claude/skills/oxikube-contributor/references/code-review.md`. It runs locally in Claude Code; there is no CI review job and no API key involved.
