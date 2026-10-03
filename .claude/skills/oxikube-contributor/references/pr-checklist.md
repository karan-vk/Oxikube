# PR checklist (author fills, reviewer verifies)

## Scope
- [ ] PR implements exactly one story; title carries the story ID.
- [ ] Every acceptance criterion is addressed and the PR says how.
- [ ] No unrelated refactors; follow-ups are filed as issues.

## Architecture
- [ ] `cargo xtask lint-deps` passes; no new cross-layer edges.
- [ ] No `gpui`/`kube`/`gpui_component` in a layer that bans them.
- [ ] New external capability went through a port with a testkit fake.
- [ ] gpui-component used only via `oxikube_ui`.

## Safety
- [ ] Every mutation goes through `MutationGuard` with the tier named in the PR.
- [ ] Read-only mode blocks it (test present).
- [ ] Audit record written; secrets redacted in logs/audit.
- [ ] No credentials, decoded secrets, or scrollback persisted.

## Commands and tools
- [ ] Each user action is a `Command` registered in the bus, visible in the palette.
- [ ] A ToolDef stub exists (or is updated) for the action; mutating tools are gated.
- [ ] Keymap context and default binding added where sensible.

## Settings / theme
- [ ] New settings have defaults, schema entries, hot reload, and docs in the PR.
- [ ] Colours come from theme tokens, not literals.

## Tests
- [ ] Unit tests with fakes for app/domain changes.
- [ ] kind integration test for adapter changes (`--features integration`).
- [ ] `#[gpui::test]` for UI; screenshot where the story says so.
- [ ] Tests are deterministic (no OS threads waking tasks, no sleeps).

## Quality
- [ ] fmt, clippy -D warnings, deny, doc build clean.
- [ ] Public items documented; glossary terms from `docs/CONTEXT.md` used.
- [ ] Vendored code has licence headers + THIRD_PARTY_NOTICES entry.
- [ ] Project `Status` is `In Review`.

## Performance (ADR 0013, docs/PERFORMANCE.md)
- [ ] No I/O, process spawn or heavy lock on the UI thread; work goes through `spawn_kube`.
- [ ] Scrolling views are virtualised; notifications coalesced; no per-event full re-sort.
- [ ] Hot-path change: `--perf` p95/p99 before/after numbers in the PR; within budget.

## Code review (mandatory)
- [ ] Author ran `/code-review high` on Sonnet before opening; summary + model in the PR.
- [ ] Every CONFIRMED finding fixed; PLAUSIBLE ones fixed or justified in one line.
- [ ] Reviewer ran `/code-review high <PR#> --comment` on Sonnet; no open confirmed findings.

## Reviewer verdict
Approve only when every box is either checked or has a written justification in the PR.
