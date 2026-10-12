# Oxikube software factory

This is the operating layer for the existing story/Project/CI factory. It works with any
agent that can read files and run commands. Codex and Claude Code receive the same task
packet and check receipts. Python 3.10+, Git and (for metrics) authenticated `gh` are the
only dependencies; no model API key, daemon, paid orchestrator or Python packages are needed.

The contributor agreement remains authoritative for architecture, tests, safety and review.
The [baseline audit](2026-10-12-audit.md) explains which observed problems this layer addresses.

## Goal: hyper-fast, accurate development

Optimize for the shortest time from an accepted task to a correct, verified, integrated
result. Measure speed together with accuracy: throughput and lead time alongside review
escapes, rework and main/nightly health. More agents, commits or generated lines are useful
only when they improve that result.

- Deliver small, complete vertical slices with explicit acceptance criteria and early tests.
- Parallelize independent work in isolated worktrees; settle shared interfaces before their
  consumers diverge. Tune concurrency to observed review, build and integration capacity.
- Use targeted checks during iteration and required full gates at delivery boundaries.
  Repeat verification when changes invalidate its evidence; reuse still-valid evidence.
- Start required reviews as soon as a coherent diff is ready, resolve findings before merge,
  and prioritize unblocking completed work over starting more work that will queue.
- Keep coordination lightweight: persist decisions, next actions and results once; add a
  tool or process step only when it removes measured delay or prevents a concrete defect.

The three-PR integration-lane trial below is an adjustable experiment, not a speed ceiling.
Raise capacity when verification and review keep up; reduce contention when they do not.

## Start a story

1. Read the issue and epic. Check the live Project's lowest-wave Ready stories and native
   blockers. Claim the Project item before creating the local worktree. One active story
   per worker. The local claim protects only worktrees of **this clone**; the Project is
   the cross-machine coordination source. This tool does not change GitHub state.
2. Start from a fetched main. For an explicitly requested maintenance task without a
   story, use a descriptive task ID and record the request as its issue reference; assign
   a real issue before opening a story PR. Never invent an existing story number.

```sh
git fetch origin
python3 tools/factory/factory.py start \
  --task E07-S03 --slug resource-table \
  --worktree ../Oxikube-wt/E07-S03 \
  --owner codex-resource-table \
  --issue https://github.com/karan-vk/Oxikube/issues/109 \
  --objective 'Implement the issue acceptance criteria for the resource table'
```

This creates a branch, worktree, exclusive local claim, `.factory/task.json` and editable
`.factory/task.md`. Fill the Markdown packet with acceptance criteria, allowed modules,
dependencies, expected tests and any API contracts shared with other workers. Record the
exact base SHA. Local files are ignored, and claims live in Git's common directory.
Keep packets factual and free of secrets. Another clone needs an explicitly transferred
packet; no transcript or hidden harness memory is required.

Use `status` to see claims, handoffs and whether the handoff fingerprint is still current
across this clone. Corrupt claims are reported individually so healthy tasks remain visible.
After merge or an intentional
abandonment, run `release --reason 'merged in PR …'` from that task's worktree. It leaves
the files and branch intact. Update the Project separately. An interrupted setup can leave
a claim with no packet: inspect the claim and worktree before manually clearing it; age alone
is never permission to steal another worker's task.

## Implement and hand off

From the task worktree, print the appropriate launch command:

```sh
python3 tools/factory/factory.py prompt --harness codex
python3 tools/factory/factory.py prompt --harness claude
python3 tools/factory/factory.py prompt --harness manual
```

Run the printed command yourself or through an authorized harness launcher. `prompt` never
executes a model, disables permission checks, publishes, or merges. For older branches,
invoke the script using an absolute path from a checkout that contains it.

Codex reads `AGENTS.md`; Claude Code reads `CLAUDE.md`. Both point here and to the same
contributor skill. Other harnesses receive the manual prompt. Model choice for implementation
belongs to the caller. The repository's two Sonnet 5.5 review passes remain the release
policy: implementing with Codex does not require implementing with Claude.

At a harness switch, a blocked dependency, or the end of a work session:

```sh
python3 tools/factory/factory.py handoff --phase verify \
  --summary 'Acceptance criteria 1–3 implemented; tests listed in task.md' \
  --next 'Run the full gate, then the author review'
```

Add `--blocker 'concrete dependency or failing check'` when blocked. The handoff records HEAD
and a content fingerprint covering the dirty diff and untracked files. The receiving agent
compares these with current files, reads the packet and receipts, and resumes the next action.
The phase is a worker's progress statement, **not** an automated approval or proof of done.
Every handoff is also appended to `.factory/events.jsonl`, retaining phase changes and
blocked intervals across harness switches; `.factory/handoff.json` holds the latest entry.

## Verify and review

```sh
python3 tools/factory/factory.py check --gate fast
python3 tools/factory/factory.py check --gate full --plan
python3 tools/factory/factory.py check --gate full
```

During implementation run targeted crate tests and the fast gate. At the PR boundary run
the full contributor gate. This tool executes the commands in `tools/factory/config.json`
sequentially, stops at the first failure, and stores exit codes, duration, HEAD and before/after
fingerprints in `.factory/checks/`. It refuses to report a passing gate if files changed while
the commands ran. Receipts are evidence, not cached permission to skip checks. Additional
kind, feature, screenshot and performance tests still follow the story and CI requirements.

Use one writer per worktree; bound concurrent Rust builds to available memory and CPU.
Sharing `CARGO_TARGET_DIR` can save compilation but serializes Cargo's build lock. Prefer
one verification lane on a busy host; measure contention before launching more workers.

Review handoff:

```sh
python3 tools/factory/factory.py prompt --harness claude --role review
```

Follow `.claude/skills/oxikube-contributor/references/code-review.md`: author review before
opening the PR, then reviewer review on the PR, both on Sonnet 5.5 at medium effort. The
printed review prompt requests a local review; posting PR comments requires the separately
authorized reviewer workflow. A Codex review can supplement but cannot stand in for the
required Sonnet passes. If Claude is unavailable, leave the PR draft and record the handoff.

Before opening/merging, verify all of the following against the actual final revision:

- Every acceptance criterion has evidence, including any required visual/performance result.
- The full gate passed; report outstanding failures without checking a contradictory box.
- Author/reviewer evidence names the reviewed SHA, model, confirmed findings and resolutions.
  Material edits after review trigger a focused re-review and affected tests; a changed base
  triggers integration verification. Do not merge while a review is still running.
- Required CI checks are green on the current PR head, branch is current, reviewer approval
  exists, and confirmed findings are resolved. Keep GitHub's existing branch protection.

The CLI deliberately does not certify merge readiness or auto-merge: it cannot authenticate
a prose review, verify every acceptance criterion or replace the repository's review process.

## Measure flow

```sh
mkdir -p .factory
python3 tools/factory/factory.py metrics \
  --since 2026-10-03T00:00:00Z --until 2026-10-12T02:45:00Z \
  --save .factory/baseline.json > .factory/summary.json
python3 tools/factory/factory.py metrics --snapshot .factory/baseline.json
```

The collector only reads GitHub. It paginates all PRs, queries Actions by UTC day to avoid
the 1,000-result search cap, deduplicates run IDs, and fails rather than silently truncating
a day with 1,000+ runs. Snapshots retain operational metadata, not bodies, patches or logs.
Use explicit UTC boundaries when comparing complete periods. Percentiles use nearest rank.
The summary separates PR and push CI, scheduled and manual nightlies, pending runs and
cancellations. Read its limitations; PR-open-to-merge is not implementation lead time,
and workflow elapsed time is not runner cost. Live GitHub state can change during collection.

At each weekly planning pass, compare equal complete windows and inspect:

| Signal | Action |
|---|---|
| Oldest review-ready work or growing PR-open-to-merge p90 | Drain review/integration before starting more stories |
| Repeated failure signature on main/nightly | Give the repair an owner and a reproducer before adding affected work |
| Many superseded runs | Batch fixes locally; push at reviewable checkpoints |
| Many dependency update runs | Consider grouping compatible updates in a separate story |
| Review fixes after merge | Audit revision/timing evidence and merge ownership |

Trial a limit of three review-ready story PRs per integration lane; seven were observed in
the baseline. This is a starting experiment, not a proven optimum or a global worker limit.
When that lane is full, finish reviews and repairs. One coordinator owns the merge order;
workers rebase when their PR enters that lane rather than all chasing every new main commit.
Recheck the required current-base CI before merging. Keep quality gates unchanged while
measuring whether this reduces cancellation and PR wait time.

## Maintain the factory

`python3 tools/factory/factory.py check --gate factory` runs deterministic tests against
temporary Git repositories and mocked GitHub pages. The existing CI lint job also runs them.
The config is the CLI's gate list; the contributor rules and CI remain authoritative. Change
the config when that policy changes. Harness-specific flags stay in `prompt`, not in story
state or the contributor contract. Metrics and packets are versioned JSON for reuse by a
future scheduler; adding one is justified only after measuring a coordination bottleneck.
