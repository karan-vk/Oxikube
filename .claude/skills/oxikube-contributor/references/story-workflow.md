# Story workflow

## Lifecycle

1. **Choose**: pick a story whose `Depends` are Done (or whose deps you can fake). Prefer
   the lowest-numbered open story in the current phase unless the epic says otherwise.
2. **Claim**: Project `Status` -> `In Progress`, assign yourself, set `Team`.
3. **Branch**: `git switch -c story/E07-S03-short-slug main`. Keep the story ID; the slug
   is 2-4 words, kebab-case.
4. **Build**: follow the acceptance criteria literally. Write tests alongside code.
5. **Verify**: run the full local gate (see SKILL.md section 6).
6. **PR**: title `feat(resources_ui): E07-S03 generic resource table` (type(scope): ID
   title). Fill the template. Link the issue with `Closes #<n>`. Set `Status` -> `In Review`.
7. **Review**: a reviewer agent reviews against `pr-checklist.md`. Address every comment
   or explain why not. Squash merge when CI is green and approval is given.
8. **Close**: `Status` -> `Done` (auto when the issue closes). Note follow-ups as new
   issues labelled `story` + the epic's phase label, linked to the epic.

## Commits

Conventional commits: `type(scope): summary` where type is `feat|fix|refactor|test|docs|
chore|perf|build|ci` and scope is the crate short name (`domain`, `ports`, `app`, `kube`,
`workspace`, `resources_ui`, `xtask`...). Imperative mood, <= 72 chars. Body explains why.
Do not add co-author trailers.

## Splitting a story

If a story turns out larger than ~3 days: finish a coherent vertical slice, open a PR for
it, and create a sibling story `E07-S03b` (same parent epic, `Depends: E07-S03`) for the
remainder. Comment on the original issue with the split. Never leave half-wired code
behind a TODO; use a feature flag or keep the slice self-contained.

## When a story needs a decision not in the plan

- If it is local (naming, module layout): decide, note it in the PR.
- If it changes a decision in `docs/PLAN.md` or `docs/adr/`: write an ADR
  (`docs/adr/NNNN-title.md`: Context, Decision, Consequences, Alternatives) in the same
  PR and link it from the story. ADRs are short (< 1 page).
- If it affects other teams' stories (a port signature change): open the ADR first as its
  own small PR so dependents see it before you build on it.

## PR template fields

The template asks for: story ID + link, summary, how each acceptance criterion is met,
guard tier for any mutation, commands/tools added, settings/keymaps added, tests added and
how to run them, screenshots for UI, vendored code and licence notes, follow-ups filed.
