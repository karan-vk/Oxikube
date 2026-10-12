# The story factory

Oxikube epics are implemented by the `oxikube-epics` workflow
(`.claude/workflows/oxikube-epics.js`), run from Claude Code with ultracode on. It turns a
config of stories into a dependency DAG. Each story then goes through these stages:

```
warm (once) -> triage -> implement -> simplify -> author review (3 lenses x 2 skeptics) -> fix
  -> PR -> reviewer pass (+ skeptic) -> serial merge -> main health (once)
```

Each stage follows the contributor skill (`.claude/skills/oxikube-contributor/`). The workflow
only orchestrates.

## Running it

Ask Claude Code to run the `oxikube-epics` workflow and pass the config JSON as `args`:

```json
{
  "main": "/absolute/path/to/Oxikube",
  "worktrees": "/absolute/path/to/Oxikube-wt",
  "epics": [{ "id": "E14", "issue": 17, "name": "Events", "guidance": "...", "lens_extra": "..." }],
  "stories": [
    { "id": "E14-S01", "epic": "E14", "issue": 300, "title": "...", "branch": "story/E14-S01-slug",
      "deps": [], "model": "sonnet", "scope": "events_ui", "labels": "phase:2,area:ui",
      "module": "oxikube_events_ui::...", "state": "new" }
  ],
  "common": "optional guidance appended to every story's context",
  "fast_merge": [],
  "fast_merge_issue": 675
}
```

- `main` is required. `worktrees` defaults to `<main>-wt`.
- `model`: `sonnet` for most stories, `opus` for genuinely hard ones (concurrency core,
  integration suites). If a Sonnet step fails, it is retried on Opus and the PR says so.
- `deps` may name stories from other epics in the same run. A story waits only for the
  dependencies that are in the config.
- `fast_merge` lists stories that merge right after a rebase and `cargo check`, without
  waiting for CI. Failures go to `fast_merge_issue`. Use it only when the repo owner asks.

## Rerunning and resuming

You don't need to build a "remaining stories" config. The Triage stage reads GitHub for every
story:

| GitHub state | What the run does |
|---|---|
| PR merged | skips the story; dependents go ahead |
| PR open, reviewer pass finished (review posted, "pending" cleared from the PR body) | merges it |
| PR open, reviewer pass missing or unfinished | runs the reviewer pass, then merges |
| no PR | full pipeline; an existing branch or worktree is picked up and finished |

If triage fails twice, the story stops with `triage failed`; it is never treated as "no PR".
Rerun the same config to continue a stopped run. Use `resumeFromRunId` only within the same
user turn: a resume from a later turn misses the agent cache.

## Why it is shaped for throughput

`main` requires PR branches to be up to date and CI to be green on that exact head. So merges
are serial, and each one costs a full CI run. The run's critical path is about
`stories x CI time`, so the workflow and CI keep that product small:

- **Warm.** One agent builds the main checkout's `target/` (clippy + `test --no-run`) before
  any story starts. Each story worktree then clones it with `cp -cR` (APFS copy-on-write),
  instead of every parallel agent doing its own cold GPUI build.
- **Merge.** The merge step rebases, runs a quick gate (fmt, lint-deps,
  `cargo check --workspace --all-targets`), and pushes. CI then starts right away, and the
  full local gate runs alongside it, so a local failure shows before CI's and the gate adds no
  wait unless it is slower than CI.
- **CI.** Only `main` saves Rust caches; PRs restore them. Before, PR caches pushed main's
  out of the 10 GB quota, so most runs compiled about 950 crates from scratch. The build legs
  no longer wait for the lint job. `cargo test` runs with `--no-fail-fast`. The xtask test that
  rebuilt the app binary (60–125 s) is opt-in, because `gen-settings-schema --check` already
  covers it. Runs on `main` are never cancelled by a newer push: a cancelled run saves no
  cache.
- **Dependabot.** Minor and patch updates come as one grouped PR a week (majors stay separate),
  with no automatic rebase. Story PRs no
  longer queue behind four dependency PRs re-running CI on every merge.
- **Main health.** After the last merge, one agent waits for main's CI and fixes any
  failures. Fast merges and cross-story test drift then get caught the same day (#686).

## Board updates

`scripts/factory/setstatus.sh <issue> <Backlog|Ready|In Progress|In Review|Done> [team]` moves
a story on project 2 and stamps its Team field. The workflow calls it with
`ultracode-<epic ids>`.
