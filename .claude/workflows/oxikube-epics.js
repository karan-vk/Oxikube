export const meta = {
  name: 'oxikube-epics',
  description: 'Implement Oxikube epics as a story DAG: warm cache -> triage -> implement -> simplify -> author review (3 lenses + skeptics) -> fix -> PR -> reviewer pass -> serial merge -> main health',
  whenToUse: 'Implementing one or more Oxikube epics end to end; args = config JSON (see scripts/factory/README.md)',
  phases: [
    { title: 'Warm', detail: 'build the main checkout once so every story worktree clones a warm target/' },
    { title: 'Triage', detail: 'read each story GitHub state so a rerun skips merged work', model: 'haiku' },
    { title: 'Implement', detail: 'finish or build each story in its worktree (Opus for hard stories)' },
    { title: 'Simplify', detail: 'Sonnet simplification pass before review', model: 'sonnet' },
    { title: 'Author review', detail: 'inline /code-review medium recipe, 3 lenses, Sonnet', model: 'sonnet' },
    { title: 'Verify', detail: 'adversarial skeptics per finding, Sonnet', model: 'sonnet' },
    { title: 'Fix', detail: 'resolve upheld findings' },
    { title: 'PR', detail: 'open PR, reviewer pass on PR', model: 'sonnet' },
    { title: 'Merge', detail: 'serialized rebase + push, local gate alongside CI, squash merge', model: 'sonnet' },
    { title: 'Main health', detail: 'confirm main is green after the last merge; fix it if not', model: 'sonnet' },
  ],
}

// args (scripts/factory/README.md): { main, worktrees?, epics: [{id, issue, name, guidance, lens_extra}],
//   stories: [{id, epic, issue, title, branch, deps, model, scope, labels, module, state, pr?}], common?, fast_merge? }
const MAIN = args.main
if (!MAIN) throw new Error('args.main (absolute path of the main checkout) is required')
const WT = args.worktrees || MAIN + '-wt'
const SETSTATUS = `${MAIN}/scripts/factory/setstatus.sh`
const EPICS = args.epics || [args.epic]
const TEAM = 'ultracode-' + EPICS.map(e => e.id).join('+')
const EPIC_BY = {}
for (const e of EPICS) EPIC_BY[e.id] = e
const E = (s) => EPIC_BY[s.epic] || EPICS[0]
const STORIES = args.stories
const BY_ID = {}
for (const s of STORIES) BY_ID[s.id] = s
const deviations = []

function ctx(s) {
  const EPIC = E(s)
  return `You are working on Oxikube epic ${EPIC.id} (GitHub issue #${EPIC.issue}, "${EPIC.name}"), story ${s.id} "${s.title}" (issue #${s.issue}).
Repo: karan-vk/Oxikube. Main checkout: ${MAIN} (stays on branch main; NEVER edit, commit or switch branches there).
Story worktree: ${WT}/${s.id}  Branch: ${s.branch}  (do all work inside the worktree, with absolute paths).
Other worktrees under ${WT}/ belong to parallel agents: never touch them.
Before coding, read the contributor skill at ${MAIN}/.claude/skills/oxikube-contributor/SKILL.md and the references you need (testing.md, architecture-rules.md, story-workflow.md, gpui-gotchas.md for any GPUI code, zed-vendoring.md when copying from Zed). Read the story's acceptance criteria with \`gh issue view ${s.issue}\` and the epic with \`gh issue view ${EPIC.issue}\`. The ACs are the spec. Read docs/ARCHITECTURE.md and the existing code of the crates you touch and reuse what exists.
House rules: no god files (directory module per area, roughly 350 lines of non-test code per file at most, thin crate root); hexagonal layer rules (\`cargo xtask lint-deps\`); views use gpui-component only via oxikube_ui; every user action is a Command in the CommandBus with an MCP tool stub; every mutation through MutationGuard; no secrets on disk or in logs; no self-dropping GPUI Tasks, Kubernetes work via oxikube_runtime::spawn_kube, nothing blocking on the UI thread, coalesced notify; vendored Zed code keeps the GPL header + THIRD_PARTY_NOTICES entry; settings get defaults in default.json + schema + hot reload; rustdoc on new public items; conventional commits "type(${s.scope}): ..." with NO co-author trailers; no out-of-scope changes.
Epic-specific guidance: ${EPIC.guidance}${args.common ? ' ' + args.common : ''}
Builds: iterate with crate-scoped commands (\`cargo test -p <crate>\`, \`cargo clippy -p <crate> --all-targets -- -D warnings\`). Long cargo commands: use a Bash timeout of 600000 ms or run_in_background. If an error names a path under another worktree (stale CARGO_MANIFEST_DIR in a copied target dir), \`touch\` that crate's src/*.rs and rebuild.
Kind cluster (only if the story needs one): context kind-oxikube is up. Run integration tests with \`OXIKUBE_TEST_CONTEXT=kind-oxikube cargo test -p <crate> --features integration --test <name>\`. Other agents use this single-node cluster concurrently: own random namespace per test, unique names for cluster-scoped objects, never cordon/taint/drain the real node, never delete shared fixtures.`
}

const IMPL_SCHEMA = {
  type: 'object',
  properties: {
    status: { type: 'string', enum: ['done', 'blocked'] },
    summary: { type: 'string', description: 'what the story adds, module layout' },
    ac_mapping: { type: 'string', description: 'markdown: each AC -> where implemented + which tests prove it' },
    tests: { type: 'string', description: 'markdown: unit/gpui/integration tests, how to run, results' },
    interpretations: { type: 'string', description: 'markdown: interpretation choices (also commented on the issue)' },
    perf: { type: 'string', description: 'markdown perf numbers if hot path or startup, else empty' },
    outside_edits: { type: 'string', description: 'small edits outside the story module and why' },
    blocker: { type: 'string' },
  },
  required: ['status', 'summary', 'ac_mapping', 'tests', 'interpretations', 'perf', 'outside_edits', 'blocker'],
}
const SIMP_SCHEMA = {
  type: 'object',
  properties: { changes: { type: 'array', items: { type: 'string' } }, gate_green: { type: 'boolean' }, notes: { type: 'string' } },
  required: ['changes', 'gate_green', 'notes'],
}
const FINDINGS_SCHEMA = {
  type: 'object',
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          title: { type: 'string' }, file: { type: 'string' }, line: { type: 'integer' },
          severity: { type: 'string', enum: ['high', 'medium', 'low'] },
          description: { type: 'string' }, failure_scenario: { type: 'string' },
        },
        required: ['title', 'file', 'line', 'severity', 'description', 'failure_scenario'],
      },
    },
  },
  required: ['findings'],
}
const VERDICT_SCHEMA = {
  type: 'object',
  properties: { refuted: { type: 'boolean' }, reasoning: { type: 'string' } },
  required: ['refuted', 'reasoning'],
}
const FIX_SCHEMA = {
  type: 'object',
  properties: {
    resolutions: { type: 'array', items: { type: 'object', properties: { title: { type: 'string' }, resolution: { type: 'string' } }, required: ['title', 'resolution'] } },
    gate_green: { type: 'boolean' }, notes: { type: 'string' },
  },
  required: ['resolutions', 'gate_green', 'notes'],
}
const PR_SCHEMA = {
  type: 'object',
  properties: { pr_number: { type: 'integer' }, url: { type: 'string' }, notes: { type: 'string' } },
  required: ['pr_number', 'url', 'notes'],
}
const MERGE_SCHEMA = {
  type: 'object',
  properties: { merged: { type: 'boolean' }, merge_sha: { type: 'string' }, notes: { type: 'string' } },
  required: ['merged', 'merge_sha', 'notes'],
}

async function ag(prompt, opts) {
  let r = await agent(prompt, opts)
  if (r == null && opts.model === 'sonnet') {
    log(`${opts.label}: Sonnet agent failed, retrying on Opus`)
    deviations.push(opts.label)
    r = await agent(prompt + '\n\n(Note: this step was meant to run on Sonnet 5.5 but Sonnet failed; you are the Opus fallback. Record that deviation where the step writes to a PR.)', { ...opts, model: 'opus', label: opts.label + ' (opus fallback)' })
  }
  return r
}

let mergeLock = Promise.resolve()
function serial(fn) {
  const p = mergeLock.then(fn, fn)
  mergeLock = p.then(() => {}, () => {})
  return p
}

const GATE = `story gate: \`cargo fmt --all\`, clippy -D warnings on every touched crate (\`--all-targets\`, plus \`--features integration\` where the crate has it), \`cargo test\` on touched crates (incl. #[gpui::test] tests), \`cargo xtask lint-deps\`, and the story's integration/screenshot tests if any`

const implPrompt = (s, extra) => `${ctx(s)}

TASK: implement story ${s.id} completely so that every acceptance criterion is demonstrably met, with tests.
1. Setup. \`git -C ${MAIN} fetch origin\`. If ${WT}/${s.id} does not exist: \`git -C ${MAIN} worktree add ${WT}/${s.id} -b ${s.branch} origin/main\` (if the branch already exists locally or on origin, check it out instead of recreating it), then seed its build cache with an APFS clone \`cp -cR ${MAIN}/target ${WT}/${s.id}/target\` (instant, copy-on-write; the Warm phase built it, so never start a cold build of the whole workspace). If the worktree/branch already exists it holds earlier partial work (${s.state}): commit any uncommitted work as WIP, rebase onto origin/main (resolve conflicts), then assess that work critically against the ACs and finish it; keep what is right, fix what is wrong. If an open PR already exists for the branch, keep it (later steps update it).
2. Set the board: \`${SETSTATUS} ${s.issue} "In Progress" ${TEAM}\`.
3. Implement where the plan says (${s.module}). Anything user-visible must be REACHABLE IN THE RUNNING APP (\`cargo run -p oxikube\`): mount it in bins/oxikube / the cluster tab / the sidebar as the ACs imply, not only in tests with fakes; launch the real binary (debug build from your worktree; kill ONLY the PID you started, never pkill/killall by name, the user may have their own oxikube open) to confirm, and say in the result how a user reaches the feature. Port/domain changes only if an AC truly needs them, minimal, with testkit fakes updated.
4. Tests per references/testing.md: testkit fakes for app/domain, #[gpui::test] (+ screenshot where visual) for UI, kind integration (\`--features integration\`, skip cleanly when OXIKUBE_TEST_CONTEXT is unset) for adapters. Make them pass, more than once if timing-sensitive.
5. Run the ${GATE}. Update docs (ARCHITECTURE.md module table, CONTEXT.md glossary, ADR if a plan decision changes) only where the story requires it.
6. Comment your interpretation choices on the issue (\`gh issue comment ${s.issue}\`) when an AC was ambiguous.
7. Commit (\`feat(${s.scope}): ${s.id} ...\`) and push the branch (\`git push -u origin ${s.branch}\`; force-with-lease after a rebase).
If something genuinely blocks you (not just hard), return status "blocked" with the concrete blocker. Otherwise return "done".${extra || ''}`

const simplifyPrompt = (s) => `${ctx(s)}

TASK: simplification pass on story ${s.id} BEFORE code review. Quality only, not bug hunting; behaviour must not change.
Look at \`git -C ${WT}/${s.id} diff origin/main...HEAD\` and the touched files. Apply cleanups for: reuse of existing helpers instead of re-implementations (oxikube_ui, oxikube_runtime, oxikube_testkit, domain types, existing modules); duplicated code across files/tests; dead code, unused params, needless pub; over-abstraction or layers that add nothing; needless clones/allocations/locks/notifies on hot paths (render, feeds, streams); files over ~350 non-test lines (split into a directory module); unclear names vs docs/CONTEXT.md glossary; comments that restate code. Do not touch code outside the story's diff.
Then rerun the ${GATE}. Commit as "refactor(${s.scope}): ${s.id} simplify" and push (force-with-lease if needed). Return the list of changes (empty if nothing worth changing) and whether the gate is green.`

const LENSES = [
  { key: 'correctness', text: `CORRECTNESS: logic errors, wrong API semantics (__LENS_EXTRA__), concurrency and cancellation bugs (tasks not aborted on drop, self-dropping GPUI Tasks, unbounded channels, locks across await, races, re-entrancy in entity updates), error handling, resource leaks, panics/unwraps on external data.` },
  { key: 'spec-rules', text: 'SPEC + RULES: every acceptance criterion of the story issue met literally (name any that is missing or only partially met); the eleven Oxikube non-negotiables (layers, gpui/kube/gpui_component placement, MutationGuard, Command + MCP stub for user actions, no secrets, GPUI pins, spawn_kube/abort-on-drop, vendored licence headers, tests shipped, perf budgets), settings defaults/schema/hot reload, no god files (>~350 non-test lines), rustdoc on public items, no out-of-scope changes. Also: is every user-visible feature actually reachable from the running binary (bins/oxikube wiring, cluster tab, sidebar), or does it exist only in tests with fakes? Unreachable user-facing work is a finding.' },
  { key: 'tests-perf', text: 'TESTS + PERF: do tests actually assert the ACs (vacuous asserts, tests that pass when the feature is broken), flaky timing or OS-thread non-determinism in gpui tests, missing error-path tests, integration tests that leak or collide; hot-path/UI-thread cost (work in render, per-frame allocations, un-coalesced notify, blocking IO on the main thread, non-virtualised lists) against docs/PERFORMANCE.md budgets.' },
]

const lensPrompt = (s, lens, target) => `${ctx(s)}

TASK: you are a code reviewer running the inline \`/code-review medium\` recipe (read-only; do NOT edit files, commit or push). Review ${target}.
Read the full diff, every touched file in full, the story ACs (\`gh issue view ${s.issue}\`) and the contributor rules. Focus lens: ${lens.text.replace('__LENS_EXTRA__', E(s).lens_extra)}
Report only verified, high-confidence defects you can point to with a concrete failure scenario (inputs/state -> wrong result). Medium effort: few, real findings; no style nits; no speculative "consider" items. Empty list if clean.`

const skepticPrompt = (s, f, target) => `${ctx(s)}

TASK: adversarially verify one code-review finding on ${target} (read-only; do not edit). Try to REFUTE it: read the code at the cited location and around it, check the actual library behaviour (gpui / gpui-component / kube sources under ~/.cargo/registry/src or ~/.cargo/git if needed), check whether tests or other code already handle it, and whether the failure scenario can really happen. Default to refuted=true if after investigation you cannot convince yourself the defect is real.
Finding: ${JSON.stringify(f)}`

function dedupe(fs) {
  const seen = new Set(), out = []
  for (const f of fs) {
    const k = `${f.file}:${Math.round(f.line / 5)}:${f.title.toLowerCase().slice(0, 40)}`
    if (seen.has(k)) continue
    seen.add(k); out.push(f)
  }
  return out
}

async function verify(s, findings, target, votes, phaseName) {
  const judged = await parallel(findings.map((f, i) => () =>
    parallel(Array.from({ length: votes }, (_, v) => () =>
      ag(skepticPrompt(s, f, target), { label: `skeptic:${s.id}:${i}.${v}`, phase: phaseName, model: 'sonnet', schema: VERDICT_SCHEMA })))
      .then(vs => {
        const ok = vs.filter(Boolean)
        const refutedAll = ok.length > 0 && ok.every(v => v.refuted)
        return { ...f, upheld: !refutedAll, verdicts: ok.map(v => ({ refuted: v.refuted, reasoning: v.reasoning })) }
      })))
  return judged.filter(Boolean)
}

const fixPrompt = (s, upheld, where) => `${ctx(s)}

TASK: resolve these code-review findings (${where}) on story ${s.id}, each verified by skeptics. For each: fix it in code with a test that would have caught it, or, only if the fix is clearly outside the story, file a follow-up issue (\`gh issue create\` with labels story,${s.labels}, linked to epic #${E(s).issue}) and say so. Then rerun the ${GATE}, commit "fix(${s.scope}): ${s.id} address review findings", push (force-with-lease if needed).
Findings: ${JSON.stringify(upheld.map(f => ({ title: f.title, file: f.file, line: f.line, description: f.description, failure_scenario: f.failure_scenario, skeptics: f.verdicts })))}`

const prPrompt = (s, impl, simp, authorFindings, authorFix) => `${ctx(s)}

TASK: open (or update) the pull request for story ${s.id}.
1. Make sure everything is committed and pushed on ${s.branch} (\`git push -u origin ${s.branch}\`, force-with-lease after rebases).
2. Read .github/PULL_REQUEST_TEMPLATE.md and model the body on merged PR #451 (\`gh pr view 451\`), the house style: Story (Closes #${s.issue} — \`${s.id}\` ${s.title} (epic #${E(s).issue})), Summary with module layout, Acceptance criteria with checked boxes each mapped to code + tests, Safety (mutations + guard tier, read-only test, secrets), Commands/tools/settings/keymap, Tests (unit/gpui/integration + how to run + gate results; screenshots for UI), Performance, Vendored code (licence headers + THIRD_PARTY_NOTICES), Interpretations, Code review (mandatory, Sonnet), Follow-ups filed, Checklist. End the body with the line: 🤖 Generated with [Claude Code](https://claude.com/claude-code)
   Code review section must say: Author pass = ultracode workflow, inline /code-review medium recipe on Sonnet 5.5, 3 lenses (correctness, spec+rules, tests+perf) x 2 adversarial skeptics, preceded by a Sonnet simplification pass. List upheld findings with their resolution and the count refuted. Reviewer pass: "pending (posted as a PR review)". Models: implementation on ${s.model === 'opus' ? 'Opus 5.5 (hard story)' : 'Sonnet 5.5'}, review on Sonnet 5.5${deviations.length ? '; Opus fallbacks for: ' + deviations.join(', ') : ''}.
3. \`gh pr create --repo karan-vk/Oxikube --base main --head ${s.branch} --title "feat(${s.scope}): ${s.id} ${s.title}" --body-file <file>\` (another conventional type only if clearly more accurate). If a PR for the branch already exists, update its title/body instead (\`gh pr edit\`).
4. \`${SETSTATUS} ${s.issue} "In Review" ${TEAM}\`.
Data for the body:
IMPLEMENTATION: ${JSON.stringify(impl)}
SIMPLIFICATION: ${JSON.stringify(simp)}
AUTHOR REVIEW FINDINGS (with skeptic verdicts): ${JSON.stringify(authorFindings)}
AUTHOR FIX RESOLUTIONS: ${JSON.stringify(authorFix)}`

const postPrompt = (s, pr, judged, needFix) => `${ctx(s)}

TASK: publish the reviewer pass on PR #${pr} and resolve it.
1. Post ONE review with \`gh api repos/karan-vk/Oxikube/pulls/${pr}/reviews\` using event=COMMENT (you cannot approve your own PR). Body: "Reviewer pass: /code-review medium inline recipe, Sonnet 5.5 (ultracode workflow; full-PR review + adversarial skeptic per finding)." followed by the upheld findings and the refuted ones (one line each with the skeptic's reason). Put each UPHELD finding as an inline comment (comments[] with path, line, side=RIGHT) when its line is in the diff, else in the body. If there are no findings at all, post a "no findings" review.
${needFix ? `2. Fix every UPHELD finding (code change + test), rerun the ${GATE}, commit "fix(${s.scope}): ${s.id} address reviewer findings", push, and reply to each inline comment with the resolution (\`gh api repos/karan-vk/Oxikube/pulls/${pr}/comments/<id>/replies -f body=...\`).` : '2. Nothing to fix.'}
3. Update the PR body's Code review section: replace "pending" for the reviewer pass with the result (upheld/refuted counts and resolutions). Keep the rest of the body.
Findings with skeptic verdicts: ${JSON.stringify(judged)}`

const mergePrompt = (s, pr) => `${ctx(s)}

TASK: land PR #${pr} (story ${s.id}). You hold the merge lock: no other story of this run merges while you work, but other sessions may still push to main, so loop until merged. Branch protection requires the branch to be up to date with main and CI green on that exact head, so CI is the authority and every minute you hold the lock delays every other story: never run the full gate before pushing, run it alongside CI.
1. If ${WT}/${s.id} does not exist (a rerun), recreate it: \`git -C ${MAIN} fetch origin\`, \`git -C ${MAIN} worktree add ${WT}/${s.id} ${s.branch}\` (or from \`origin/${s.branch}\`), \`cp -cR ${MAIN}/target ${WT}/${s.id}/target\`. \`git -C ${WT}/${s.id} fetch origin\` and rebase ${s.branch} onto origin/main. Resolve conflicts carefully (typical: crate roots' module lists, Cargo.toml/Cargo.lock, tests/common, default.json, docs/ARCHITECTURE.md, docs/CONTEXT.md, THIRD_PARTY_NOTICES.md): keep BOTH sides' additions; regenerate Cargo.lock with cargo rather than hand-merging.
2. Quick gate (a few minutes): \`cargo fmt --all --check\`, \`cargo xtask lint-deps\`, \`cargo check --workspace --all-targets\` (catches APIs another story changed). Fix anything red (stay in scope; if the rebase brought a changed API from another story, adapt this story's code).
3. \`git push --force-with-lease origin ${s.branch}\` so CI starts at once. While it runs, run the rest of the gate locally: \`cargo clippy --workspace --all-targets -- -D warnings\` (plus \`--features integration\` on touched crates that have it), \`cargo test --workspace --no-fail-fast\` (run_in_background, 600000 ms timeout), \`cargo deny check\` if Cargo.lock changed, and this story's integration tests if any. If the local run goes red first, fix and push right away (the push cancels the stale CI run). Then \`gh pr checks ${pr} --repo karan-vk/Oxikube --watch --interval 30\` (required: "fmt + lint-deps + gpui-pin + deny", "clippy + test (ubuntu-latest)", "clippy + test (macos-latest)"). If a check fails, read the log (\`gh run view --log-failed\`), fix, push, watch again. If main moved (branch not up to date), go back to step 1.
4. Before merging confirm: the reviewer review is posted, the Code review section is complete, no unresolved findings. Then \`gh pr merge ${pr} --repo karan-vk/Oxikube --squash\` (PR title is the squash subject; strip any co-author trailers from the squash body).
5. After merge: \`${SETSTATUS} ${s.issue} "Done" ${TEAM}\`; \`git -C ${MAIN} worktree remove --force ${WT}/${s.id}\` and \`git -C ${MAIN} branch -D ${s.branch}\` (only after confirming the PR state is MERGED). Do not touch the main checkout's working tree otherwise.
Return merged=true with the squash commit sha, or merged=false with what is wrong.${fastMerge(s, pr)}`

const FAST = new Set((args && args.fast_merge) || [])
const FAST_ISSUE = (args && args.fast_merge_issue) || 675
const fastMerge = (s, pr) => FAST.has(s.id) ? `

FAST-MERGE OVERRIDE (the repo owner listed this story in args.fast_merge: merge now, fix breakage later). This replaces steps 2-4 above:
- After the rebase, run only \`cargo fmt --all\` and \`cargo check --workspace --all-targets\`. If the check fails, make the smallest fix that makes it compile. Main must always compile.
- Commit, run \`git push --force-with-lease origin ${s.branch}\`, and do NOT wait for CI. Merge right away with \`gh pr merge ${pr} --repo karan-vk/Oxikube --squash --admin\` (PR title as the subject, no co-author trailers). Do not skip the merge because the reviewer findings, the Code review section or CI checks are incomplete.
- Then run \`gh pr checks ${pr} --repo karan-vk/Oxikube\` once. If any check is red or still pending, or you skipped anything, add one comment to issue #${FAST_ISSUE} (\`gh issue comment ${FAST_ISSUE} --repo karan-vk/Oxikube --body ...\`) naming the story, PR #${pr}, each red or pending check, and any reviewer finding you know is still open. Do not try to fix CI failures now.
- Step 5 (board Done, worktree and branch cleanup) still applies.` : ''

async function land(s, prNumber, rec) {
  let m = await serial(() => ag(mergePrompt(s, prNumber), { label: `merge:${s.id}`, phase: 'Merge', model: 'sonnet', schema: MERGE_SCHEMA }))
  if (!m || !m.merged) {
    log(`${s.id}: merge attempt failed (${m ? m.notes : 'agent failed'}); retrying on Opus`)
    m = await serial(() => agent(mergePrompt(s, prNumber) + `\n\nPrevious attempt failed: ${m ? m.notes : 'agent failure'}`, { label: `merge:${s.id} (opus retry)`, phase: 'Merge', model: 'opus', schema: MERGE_SCHEMA }))
  }
  rec.merged = !!(m && m.merged)
  rec.merge = m
  return rec
}

const TRIAGE_SCHEMA = {
  type: 'object',
  properties: {
    state: { type: 'string', enum: ['merged', 'reviewed', 'pr_open', 'none'] },
    pr: { type: 'integer', description: 'open or merged PR number, 0 if none' },
    notes: { type: 'string' },
  },
  required: ['state', 'pr', 'notes'],
}

// Reruns and resumes read GitHub instead of a hand-built "remaining stories" args file: merged
// stories are skipped, open PRs pick up where they stopped. A workflow resume from a different
// user turn misses the agent cache, so this is what makes restarting a run cheap.
const triagePrompt = (s) => `Read-only GitHub lookup for story ${s.id} (issue #${s.issue}, branch ${s.branch}) in karan-vk/Oxikube. Do not edit anything.
Run \`gh pr list --repo karan-vk/Oxikube --head ${s.branch} --state all --json number,state,mergedAt\`${s.pr ? ` and also look at PR #${s.pr}` : ''}.
- A MERGED PR for the branch -> state "merged".
- An OPEN PR: run \`gh api repos/karan-vk/Oxikube/pulls/<n>/reviews --jq '.[].body'\` and \`gh pr view <n> --repo karan-vk/Oxikube --json body --jq .body\`. If a review body starts with "Reviewer pass" AND the PR body no longer says the reviewer pass is "pending" (the reviewer pass clears that only after its fixes are pushed) -> "reviewed", else "pr_open".
- No PR (or only closed, unmerged ones) -> "none".
Return the PR number (0 if none).`

const warmed = (async () => {
  return agent(`Warm the shared Cargo build cache for the Oxikube story worktrees. Main checkout: ${MAIN} (on branch main; never edit, commit or switch branches there).
1. If \`git -C ${MAIN} status --porcelain\` is empty, \`git -C ${MAIN} pull --ff-only\`; otherwise leave the checkout as it is.
2. In ${MAIN}, run \`cargo clippy --workspace --all-targets\` and then \`cargo test --workspace --no-run\` (Bash timeout 600000 ms; repeat with run_in_background if one call is not enough). Warnings and test failures do not matter here; only the build artifacts in ${MAIN}/target do. Story agents clone that directory (\`cp -cR\`), so a warm one saves each of them a cold build of the whole workspace.
Return one line: how long it took and whether ${MAIN}/target now exists.`, { label: 'warm:target', phase: 'Warm', effort: 'low' })
})()

async function reviewerPass(s, prNumber) {
  const prTarget = `PR #${prNumber} (\`gh pr diff ${prNumber} --repo karan-vk/Oxikube\`; the code is also checked out in ${WT}/${s.id})`
  const rev = await ag(lensPrompt(s, { text: 'FULL REVIEW (reviewer pass): correctness, spec/ACs, contributor rules, tests, perf. Also check the PR body is accurate and the author pass was really done.' }, prTarget),
    { label: `reviewer:${s.id}`, phase: 'PR', model: 'sonnet', schema: FINDINGS_SCHEMA })
  const revJudged = await verify(s, dedupe(rev ? rev.findings : []), prTarget, 1, 'Verify')
  const revUpheld = revJudged.filter(f => f.upheld)
  log(`${s.id}: reviewer pass ${revUpheld.length} upheld / ${revJudged.length - revUpheld.length} refuted`)
  await ag(postPrompt(s, prNumber, revJudged.map(f => ({ title: f.title, file: f.file, line: f.line, description: f.description, upheld: f.upheld, verdicts: f.verdicts })), revUpheld.length > 0),
    { label: `review-post:${s.id}`, phase: 'Fix', model: revUpheld.length ? s.model : 'sonnet' })
  return { upheld: revUpheld.length, refuted: revJudged.length - revUpheld.length }
}

async function storyChain(s, triage) {
  const rec = { id: s.id, issue: s.issue, merged: false }
  if (triage.pr && (triage.state === 'reviewed' || triage.state === 'pr_open')) {
    rec.pr = triage.pr
    // An open PR already had its author pass (the PR is opened after it); only the reviewer pass may be missing.
    if (triage.state === 'pr_open') rec.reviewer = await reviewerPass(s, triage.pr)
    return land(s, triage.pr, rec)
  }
  // The legacy `pr` field still means "reviewed, merge it".
  if (s.pr) { rec.pr = s.pr; return land(s, s.pr, rec) }
  await warmed
  let impl = await ag(implPrompt(s), { label: `impl:${s.id}`, phase: 'Implement', model: s.model, schema: IMPL_SCHEMA })
  if ((!impl || impl.status !== 'done') && s.model === 'sonnet') {
    log(`${s.id}: Sonnet implementation blocked (${impl ? impl.blocker : 'agent failed'}); escalating to Opus`)
    deviations.push(`impl:${s.id}`)
    impl = await agent(implPrompt(s, `\n\nA previous Sonnet attempt reported: ${impl ? impl.blocker : 'agent failure'}. Continue from the worktree state and resolve it.`), { label: `impl:${s.id} (opus escalation)`, phase: 'Implement', model: 'opus', schema: IMPL_SCHEMA })
  }
  if (!impl || impl.status !== 'done') { rec.reason = 'implementation blocked: ' + (impl ? impl.blocker : 'agent failed'); return rec }
  rec.impl = impl.summary

  const simp = await ag(simplifyPrompt(s), { label: `simplify:${s.id}`, phase: 'Simplify', model: 'sonnet', schema: SIMP_SCHEMA })

  const target = `the branch ${s.branch} in ${WT}/${s.id} against origin/main (\`git diff origin/main...HEAD\`)`
  const lensResults = await parallel(LENSES.map(l => () =>
    ag(lensPrompt(s, l, target), { label: `review:${s.id}:${l.key}`, phase: 'Author review', model: 'sonnet', schema: FINDINGS_SCHEMA })))
  const authorFindings = dedupe(lensResults.filter(Boolean).flatMap(r => r.findings))
  const authorJudged = await verify(s, authorFindings, target, 2, 'Verify')
  const authorUpheld = authorJudged.filter(f => f.upheld)
  log(`${s.id}: author pass ${authorUpheld.length} upheld / ${authorJudged.length - authorUpheld.length} refuted`)
  rec.author = { upheld: authorUpheld.length, refuted: authorJudged.length - authorUpheld.length }

  let authorFix = null
  if (authorUpheld.length) {
    authorFix = await ag(fixPrompt(s, authorUpheld, 'author pass'), { label: `fix:${s.id}`, phase: 'Fix', model: s.model, schema: FIX_SCHEMA })
  }

  const pr = await ag(prPrompt(s, impl, simp, authorJudged.map(f => ({ title: f.title, file: f.file, line: f.line, upheld: f.upheld, verdicts: f.verdicts })), authorFix),
    { label: `pr:${s.id}`, phase: 'PR', model: 'sonnet', schema: PR_SCHEMA })
  if (!pr || !pr.pr_number) { rec.reason = 'PR not opened'; return rec }
  rec.pr = pr.pr_number

  rec.reviewer = await reviewerPass(s, pr.pr_number)
  return land(s, pr.pr_number, rec)
}

const runs = {}
function run(id) {
  if (!runs[id]) {
    runs[id] = (async () => {
      const s = BY_ID[id]
      // Triage before waiting on dependencies: a merged story never waits.
      const triageOpts = { label: `triage:${s.id}`, phase: 'Triage', model: 'haiku', effort: 'low', schema: TRIAGE_SCHEMA }
      const triage = (await agent(triagePrompt(s), triageOpts)) || (await agent(triagePrompt(s), { ...triageOpts, model: 'sonnet', label: triageOpts.label + ' (retry)' }))
      // Unknown state: guessing "no PR" could re-implement a merged story, so stop here instead.
      if (!triage) return { id, issue: s.issue, merged: false, reason: 'triage failed' }
      if (triage && triage.state === 'merged') {
        log(`${id}: already merged (PR #${triage.pr}), skipping`)
        return { id, issue: s.issue, merged: true, pr: triage.pr, reason: 'already merged' }
      }
      const deps = await Promise.all(s.deps.filter(d => BY_ID[d]).map(run))
      const bad = deps.filter(d => !d || !d.merged)
      if (bad.length) {
        log(`${id}: skipped, dependency not merged (${bad.map(d => d && d.id).join(', ')})`)
        return { id, issue: s.issue, merged: false, reason: 'dependency not merged' }
      }
      try { return await storyChain(s, triage) } catch (e) { return { id, issue: s.issue, merged: false, reason: 'error: ' + String(e) } }
    })()
  }
  return runs[id]
}

const results = await Promise.all(STORIES.map(s => run(s.id)))

// CI's `cargo test` on a PR only proves that PR against the main it was rebased on; fast merges
// skip even that. Check the main that the run left behind and repair it at once (lesson of #686).
let mainHealth = null
if (results.some(r => r && r.merged && r.reason !== 'already merged')) {
  phase('Main health')
  mainHealth = await ag(`Check that main of karan-vk/Oxikube is green after this run's merges. Main checkout: ${MAIN} (never edit, commit or switch branches there).
1. \`gh run list --repo karan-vk/Oxikube --workflow ci.yml --branch main --limit 1 --json databaseId,status,conclusion,headSha\`; if it is not finished, \`gh run watch <id> --repo karan-vk/Oxikube --exit-status\`.
2. If it is green, return "green" with the sha.
3. If it is red: \`gh run view <id> --repo karan-vk/Oxikube --log-failed\`, \`git -C ${MAIN} fetch origin\` (the merges happened on GitHub), create a worktree \`git -C ${MAIN} worktree add ${WT}/main-health -b fix/main-health-<short sha> origin/main\` (seed it with \`cp -cR ${MAIN}/target ${WT}/main-health/target\`), fix every failure (the CI test step uses --no-fail-fast, so the log lists all of them), run the failing crates' tests and \`cargo clippy --workspace --all-targets -- -D warnings\`, commit "fix: get main green after ${TEAM}", push, open a PR, watch its checks, and merge it with \`gh pr merge --squash\` once green. Then remove the worktree. Return what was red and the PR number.`,
    { label: 'main-health', phase: 'Main health', model: 'sonnet' })
}
return { epics: EPICS.map(e => e.id), results, deviations, mainHealth }
