# GitHub Project conventions

Project: "Oxikube" (owner `karan-vk`). Repo: `karan-vk/Oxikube`.

## Issue types
- **Epic** (`epic` label): `E01`..`E29`, title `E07 - Resource browser: generic table + detail`.
  Body: goal, in/out of scope, crates, depends on, done when, risks, story table.
- **Story** (`story` label): native sub-issue of its epic, title `E07-S03 Generic ResourceTable view`.
  Body: acceptance criteria, crates/modules, depends, size.
- **Bug** (`bug` label): from the bug template; attach to the epic that owns the area.

## Fields
| Field | Values | Who sets |
|---|---|---|
| Status | Backlog, Ready, In Progress, In Review, Done | you (claim/review), automation on close |
| Phase | 0 Foundation, 1 Core k8s, 2 Lens parity, 3 Platform, 4 Integrations, 5 Agents, 6 Later | planner |
| Area | domain, ports, app, adapters, platform, ui, bins, tooling | planner |
| Priority | P0..P3 | planner / maintainer |
| Size | S (<=1 day), M (1-2 days), L (2-3 days) | planner; re-estimate in a comment if wrong |
| Team | free text | you |

Labels mirror Phase (`phase:N`), Area (`area:*`), Size (`size:*`) so `gh issue list` can
filter without the Project API. Milestones `M0`..`M6` correspond to phases.

## Finding dependencies and dependents
- A story's `Depends` column lists story IDs (`S04` = same epic, `E06-S02` = cross-epic)
  or epic IDs (the whole epic must be Done).
- To find what depends on you: `gh issue list --search "E07-S03 in:body" --label story`.
- To see an epic's remaining work: open the epic issue; the sub-issue progress bar shows
  it, or `gh issue list --search "E07-S" --state open`.

## Views
- **Board by Status**: daily pickup. Take from `Ready`, never from `Backlog` (Backlog means
  dependencies are not Done).
- **Roadmap by Phase**: planning; do not reorder phases without an ADR.
- **Table by Area**: find all open work in your crate layer.

## Etiquette
- One story In Progress per agent at a time.
- Comment on the issue when you deviate from acceptance criteria, split, or get blocked
  (`blocked` label + what unblocks it).
- Never close an epic manually; it closes when all stories are Done and the "done when"
  criteria are checked off by a maintainer.

## Dependencies and ordering

- `docs/DEPENDENCIES.md` is the dependency matrix: epic graph, per-epic story graphs (Mermaid),
  execution waves and the critical path. Stories on the critical path are marked ⚠️.
- Every story's issue lists **Blocked by** and **Unblocks** with links, and has a neighbourhood
  diagram. The same edges are native GitHub relationships, so the issue sidebar and Project
  show "Blocked".
- The Project field `Wave` gives the earliest parallel batch a story belongs to. Pick the
  lowest-wave `Ready` story in your area. A story whose blockers are not done can still start
  against `oxikube_testkit` fakes if the blocker is only a port or type. Say so in the PR.
- When you change a story's dependencies, update the issue, the native relationship and
  `docs/PLAN.md`, and regenerate `docs/DEPENDENCIES.md` (the `cargo xtask deps` follow-up
  replaces the planning scripts).

## Repository settings and branch protection

Applied on `karan-vk/Oxikube` (verified 2026-10-03 with
`gh api repos/karan-vk/Oxikube/branches/main/protection` and `gh api repos/karan-vk/Oxikube`).
These are maintainer settings; agents cannot and must not change them in a story.

**Merge method: squash only.** `allow_squash_merge=true`, `allow_merge_commit=false`,
`allow_rebase_merge=false`, `delete_branch_on_merge=true` (squash commit title and message come
from the PR title and commit messages, so the PR title must be a conventional commit). To
re-apply: `gh repo edit karan-vk/Oxikube --enable-squash-merge --enable-merge-commit=false --enable-rebase-merge=false --delete-branch-on-merge`.

**Branch protection on `main`:**

| Setting | Value |
|---|---|
| Required status checks | `fmt + lint-deps + gpui-pin + deny`, `clippy + test (ubuntu-latest)`, `clippy + test (macos-latest)` |
| Require branches to be up to date (`strict`) | on |
| Require linear history | on |
| Force pushes | blocked |
| Branch deletion | blocked |
| Required pull-request reviews | none in GitHub settings: the reviewer-agent approval and the `/code-review medium` pass are process rules (`pr-checklist.md`), not enforced by GitHub |
| Enforce for admins | off (the maintainer can override in an emergency; agents never do) |

The three check names are the job names GitHub shows for `.github/workflows/ci.yml`: the `lint`
job is `fmt + lint-deps + gpui-pin + deny` (ubuntu only), and the `build` job is
`clippy + test (${{ matrix.os }})` for `ubuntu-latest` and `macos-latest`. Renaming a job or
changing the matrix renames the check, which then never reports and blocks every merge: change
the workflow and this table (and the branch-protection setting) in the same PR. The `lint` job
runs on Linux only because fmt, `lint-deps`, `check-gpui-pin` and `cargo deny` are
OS-independent (deny already evaluates the macOS and Linux targets listed in `deny.toml`);
clippy and tests, which compile platform code, run on both OSes.

Other required-check workflows (`integration.yml`, `nightly.yml`, `release.yml`) are not
required checks; add a check to the table above only when its story says so.
