---
name: oxikube-contributor
description: The working agreement for the Oxikube repository (a Kubernetes desktop client in Rust + GPUI, structured as a hexagonal multi-crate workspace built by many agent teams from GitHub epics and stories). Use this skill for ANY work in this repo, even when the task looks small - implementing or reviewing a story, opening or reviewing a PR, adding or renaming a crate, touching oxikube_domain / oxikube_ports / oxikube_app / any adapter or ui crate, writing GPUI or gpui-component code, writing kube-rs code, Argo CD, Helm, extension (WASM) or ACP/MCP agent work, CI/xtask changes, or vendoring code from Zed or kdash. It tells you where code goes, which dependency directions are allowed, how mutations must be guarded, how to claim and finish a story, and what the reviewer will reject.
---

# Oxikube contributor skill

Oxikube is built by several agent teams working in parallel from a GitHub Project of
epics and stories. The only way that works is if every team makes the same structural
choices without talking to each other. This skill is that shared memory. Read it before
touching code; read the matching reference file when you enter its territory.

## 1. Pick up a story

1. Open the story issue (title starts with its ID, e.g. `E07-S03`). Read the parent epic
   (goal, scope, "done when", risks) and the story's acceptance criteria. They are the
   spec. If they are ambiguous, comment on the issue with the interpretation you chose
   and proceed; do not widen scope silently.
2. Check the story's `Depends` list. If a dependency is not `Done`, either pick another
   story or build against the testkit fakes and say so in the PR.
3. Claim it: set Project `Status` to `In Progress` and assign yourself (or your team
   name in the `Team` field). Unclaim by setting `Ready` again if you stop.
4. Branch from `main`: `story/E07-S03-short-slug`. One story = one branch = one PR.
5. Read `references/story-workflow.md` for the full lifecycle, and
   `references/github-project.md` for fields, labels and how to find dependents.

## 2. Where code goes (hexagonal layers)

```
oxikube_domain  <-  oxikube_ports  <-  oxikube_app  <-  ui/* , bins/oxikube
                          ^
                     adapters/*   (implement ports; never depend on app or ui)
platform/* (settings, keymap, theme, runtime, extension host) -> domain, ports only
```

Dependency direction is enforced by `cargo xtask lint-deps` in CI. The rules, stated
once so every team writes the same shape:

- `oxikube_domain`: pure types and pure functions. No internal deps, no I/O crates,
  no async runtime. Kubernetes objects are `Resource { meta, gvk, json }` plus typed
  view-models built from JSON. `k8s-openapi` is NOT allowed here.
- `oxikube_ports`: `async_trait` object-safe traits only, depending on domain. Every
  port has a fake in `oxikube_testkit`.
- `oxikube_app`: use-cases and services (ClusterSessionManager, ResourceStore,
  CommandBus, MutationGuard, LogService, ToolRegistry, ContextRegistry...). Depends on
  domain + ports. NO `gpui`, NO `kube`, NO adapter crates. It is plain async Rust so it
  is testable with fakes and reusable by the MCP server.
- `adapters/*` (`oxikube_kube`, `oxikube_argocd`, `oxikube_helm`, `oxikube_state_sqlite`,
  `oxikube_acp`, `oxikube_mcp`...): implement ports with their SDK. Depend on domain +
  ports + their external crates. Never on app or ui.
- `platform/*`: app infrastructure that may need gpui (settings store, keymap, theme,
  runtime bridge, extension host). Depend on domain + ports (+ gpui where unavoidable).
- `ui/*`: GPUI views. Depend on domain, ports, app, platform and `oxikube_ui`. Views
  import gpui-component ONLY through `oxikube_ui`; the lint bans a direct
  `gpui_component` dependency anywhere else so a future swap touches one crate.
- `bins/oxikube`: wires adapters into app, mounts ui, owns init order. May depend on
  everything.

Why: the domain/app core stays testable without a cluster or a window, adapters can be
replaced (kube-rs churns, Argo has three backends), and UI churn (gpui-pre snapshots
weekly) never leaks past `oxikube_ui`. Full crate map, naming and lint details:
`references/architecture-rules.md`.

## 3. The eleven non-negotiables

Reviewers reject PRs that break any of these. Each one exists because a real failure
mode was observed in the reference projects we studied.

1. **Layer rules above are absolute.** `cargo xtask lint-deps` must pass. Do not add a
   dependency edge to "just get it working"; move the code instead.
2. **No `gpui` / `kube` / `gpui_component` outside their allowed layers.** app and
   domain code must compile without either. Views use `oxikube_ui` re-exports.
3. **Every mutation goes through `MutationGuard`.** Read-only mode, confirmation tier
   (none / simple / type-the-name), server dry-run where possible, and an audit record.
   UI code never calls a mutating port method directly; the newtype in
   `oxikube_app::mutation` is the only door. This includes MCP tool invocations and
   Argo/Helm/extension actions.
4. **Every user-facing action is a `Command` in the `CommandBus` and registers an MCP
   tool definition (stub is fine until the agent phase).** The palette, keymap, context
   menus and hosted agents all dispatch the same `Command`. If you add a button that
   does something, it is a Command first and a button second.
5. **No secrets on disk.** Tokens, kubeconfig credentials, decoded Secret data, agent
   tokens: keychain (`SecretStorePort`) or memory only. Logs and audit entries pass
   through the redaction module. Never persist terminal scrollback.
6. **GPUI pins are exact and bumped together.** `gpui-pre*`, `gpui-component`,
   `gpui-base`, `gpui-kit-assets` move in one dedicated PR, verified by
   `cargo xtask check-gpui-pin`. Never add a Zed git dependency.
7. **No self-dropping GPUI `Task`s; all Kubernetes work via `oxikube_runtime::spawn_kube`
   (abort-on-drop).** Nothing blocking or network-bound on the UI thread. Coalesce
   `cx.notify()` to frame cadence for streams.
8. **Vendored code carries its licence.** Code copied from Zed gets the GPL-3.0-or-later
   header and a THIRD_PARTY_NOTICES entry; code from kdash keeps its MIT notice; deskribe
   keeps the Kubernetes NOTICE. Prefer copying Zed's *design* over its code when the code
   is entangled. See `references/zed-vendoring.md`.
9. **Story = branch = PR, reviewed by `/code-review` on Sonnet.** Conventional commits,
   PR template filled, CI green, `/code-review` run twice (author before opening, reviewer
   on the PR), one reviewer-agent approval, squash merge. No drive-by changes outside the story's
   scope; open a follow-up issue instead.
10. **Tests are part of the story.** Unit tests with `oxikube_testkit` fakes for app
    and domain code; kind integration tests (`--features integration`) for adapters;
    `#[gpui::test]` tests (and a screenshot where visual) for UI. A story without tests
    is not done.

11. **It must feel as smooth as Zed.** Budgets in `docs/PERFORMANCE.md` (ADR 0013): p95
    frame ≤ 8 ms under 10 k-pod churn, input-to-pixel ≤ 1 frame, cold start ≤ 400 ms, zero
    blocking work on the UI thread, everything that scrolls is virtualised, `cx.notify()`
    coalesced. Hot-path PRs (tables, feeds, logs, editor, terminal, agent thread) report
    `--perf` numbers before/after. Reviewers reject correct-but-janky code.

## 4. Definition of done

A story is done when all of the following hold:

- Every acceptance criterion in the issue is demonstrably met (say how, in the PR).
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `cargo xtask lint-deps`, `cargo xtask check-gpui-pin`,
  `cargo deny check` pass locally and in CI.
- New public types have rustdoc; `oxikube_domain` and `oxikube_ports` keep
  `#![deny(missing_docs)]` green.
- Settings you added have defaults in `default.json`, a schema entry, and hot reload.
- Commands you added appear in the palette with a keybinding (if sensible) and a tool
  stub; mutating ones state their guard tier in the PR.
- `/code-review high` was run on the branch (model: Sonnet 5.5) and every confirmed
  finding is fixed or justified in the PR's "Code review" section; the reviewer re-runs
  `/code-review high <PR#> --comment` before approving (see `references/code-review.md`).
- The PR checklist (`references/pr-checklist.md`) is filled, the Project `Status` is
  `In Review`, and after merge `Done`.

## 5. Reference files (read when...)

| File | Read when |
|---|---|
| `references/architecture-rules.md` | adding a crate or module, unsure which crate owns something, lint-deps fails, naming a port/service |
| `references/story-workflow.md` | starting or finishing a story, writing commits/PRs, splitting a story, writing an ADR |
| `references/pr-checklist.md` | opening or reviewing a PR |
| `references/code-review.md` | running the mandatory `/code-review` pass (author and reviewer), which model, what to do with findings |
| `references/testing.md` | writing any test, using kind, writing a gpui test, adding a screenshot test |
| `references/gpui-gotchas.md` | writing GPUI or gpui-component code, a UI test is flaky, bumping GPUI |
| `references/zed-vendoring.md` | copying anything from Zed or another project |
| `references/github-project.md` | claiming work, finding dependencies, labelling issues |

Repo-level docs: `docs/ARCHITECTURE.md` (the canonical crate map), `docs/CONTEXT.md`
(domain glossary; use its terms in type names), `docs/adr/` (decisions; add one when you
change a decision), `docs/research/` (the feature inventories and ecosystem research
behind the plan), `docs/PLAN.md` (the full epic and story catalogue).

## 6. Quick commands

```
cargo check --workspace                      # fast compile gate
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                       # unit + gpui tests
cargo xtask lint-deps                        # layer rules
cargo xtask check-gpui-pin                   # GPUI pins aligned
cargo deny check                             # licences + advisories
cargo xtask kind-up && cargo test -p oxikube_kube --features integration
cargo xtask load-pods --count 10000 --churn  # perf fixture
```
