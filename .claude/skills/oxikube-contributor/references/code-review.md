# Mandatory `/code-review` pass

Every PR in this repository is reviewed by Claude Code's `/code-review` skill, run locally
in Claude Code on Sonnet at `medium` effort, twice: once by the author before the PR opens,
once by the reviewer on the open PR. The
project owner asked for this explicitly; it is not optional and the reviewer checks for it.

## Model

Run the review on **Sonnet 5.5** (`claude-sonnet-5-5`). In an interactive session switch
first with `/model sonnet`, then invoke the skill. When a reviewer subagent is spawned to do
the review, spawn it with `model: sonnet`. Both passes run locally in Claude Code sessions; there is no CI review job and no
API key involved.

Why Sonnet: review is breadth work over a bounded diff; Sonnet is fast and cheap enough to
run on every PR without anyone skipping it, which matters more than marginal depth.

## Author pass (before opening the PR)

1. Commit everything on the story branch; make sure the gate is green
   (`cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`,
   `cargo xtask lint-deps`, `cargo xtask check-gpui-pin`, `cargo deny check`).
2. Run `/code-review medium` (reviews the current branch diff against `main`).
3. Fix every CONFIRMED finding. For PLAUSIBLE findings either fix them or write one line
   explaining why not. Re-run after fixing if the diff changed materially.
4. Paste the final findings summary (or "no findings") into the PR template's
   **Code review** section, with the model used.

## Reviewer pass (on the open PR)

1. `/code-review medium <PR number> --comment` posts findings as inline PR comments.
2. Verify the author's pass actually happened (section filled, findings addressed).
3. Walk `references/pr-checklist.md`. Approve only when every confirmed finding is
   resolved and every checklist box is checked or justified.

## Running the review from a subagent

`/code-review` called from a subagent (agent teams, Workflow agents) forks into a background
agent whose result goes to the top-level session, not the caller: the subagent never sees
findings and nothing is posted. The requirement is unchanged (twice, `medium`, Sonnet); pick
one way to run it:

1. **Top-level session.** The top-level Claude Code session, on Sonnet, runs
   `/code-review medium` (author pass) or `/code-review medium <PR#> --comment` (reviewer
   pass) itself.
2. **Inline recipe.** A subagent spawned with `model: sonnet` runs the medium recipe inline:
   read the full diff, the touched files, the story's acceptance criteria and the contributor
   rules; report only verified, high-confidence findings. Author pass: put findings in the PR
   body. Reviewer pass: post them as inline PR review comments (`gh api
   repos/<owner>/<repo>/pulls/<n>/reviews` with `event=COMMENT` and `comments[]`), or a
   "no findings" review when clean.

Either way, state in the PR's **Code review** section which way it ran (top-level `/code-review`
or inline recipe), plus the model.

## What counts as resolved

- A code change in the PR, or
- A written justification in the PR thread that a human could disagree with (not
  "won't fix"), or
- A follow-up issue linked from the PR when the fix is clearly outside the story.

Do not merge a PR with an unaddressed CONFIRMED finding. The standard effort level is
`medium`; do not go lower. Use `high` only when the diff is large or touches a hot path or a
mutation, and say so in the PR.
