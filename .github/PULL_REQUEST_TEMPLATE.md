## Story
Closes #<issue>  —  `E__-S__` <title>

## Summary
<!-- what changed and why, 2-5 lines -->

## Acceptance criteria
<!-- copy each criterion from the story and say how it is met / where the test is -->
- [ ] ...

## Safety
- Mutations introduced: <none | list> — guard tier: <none | simple | type-the-name>, dry-run: <yes/no>, audit: <yes/no>
- Read-only mode test: <path>

## Commands / tools / settings / keymap
- Commands added: ...
- Tool stubs added: ...
- Settings added (default + schema + hot reload): ...
- Keymap contexts/bindings: ...

## Tests
- Unit (fakes): ...
- Integration (kind): ...
- GPUI / screenshot: ...

## Performance
<!-- hot path touched? paste `--perf` p50/p95/p99 before/after and memory; else "not a hot path" -->

## Vendored code
<!-- source, licence, header added, THIRD_PARTY_NOTICES updated — or "none" -->

## Code review (mandatory, Sonnet)
- Author pass: `/code-review high` on `claude-sonnet-5-5` — findings: <none | summary + how each was resolved>
- Reviewer pass: `/code-review high <PR#> --comment` — <pending | done, no open confirmed findings>

## Follow-ups filed
- #...

## Checklist
- [ ] `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`
- [ ] `cargo xtask lint-deps`, `cargo xtask check-gpui-pin`, `cargo deny check`
- [ ] PR title is `type(scope): E__-S__ title`; single story; no out-of-scope changes
- [ ] `/code-review high` run on Sonnet before opening; confirmed findings resolved
- [ ] Project Status set to In Review
