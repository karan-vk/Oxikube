# ADR 0018: Google Rust Style Guide

- **Status:** Accepted (2026-10-28)
- **Deciders:** project owner
- **Related:** docs/STYLE.md (the rules as applied here), root `Cargo.toml`
  `[workspace.lints.clippy]`, `clippy.toml`, `rustfmt.toml`, ADR 0002 (hexagonal layers)

## Context

Many agent teams write Oxikube in parallel. The architecture, safety and performance rules were
written down; code style was not. The workspace lints came from Zed's manifest and turned the
whole `clippy::style` group off, so even the ecosystem's default conventions (`ptr_arg`,
`new_without_default`, `len_without_is_empty`, `wrong_self_convention`) went unchecked, and each
team settled questions such as `unwrap` versus `expect`, assertion messages and `SAFETY`
comments differently.

## Decision

Adopt the [Google Rust Style Guide](https://google.github.io/styleguide/rustguide.html), as
written down for this repo in `docs/STYLE.md`, and enforce every rule that clippy can check:

- Turn the default `clippy::style` group back on.
- Add `unwrap_used`, `missing_panics_doc`, `missing_assert_message`,
  `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`, `wildcard_imports`,
  `enum_glob_use`, `match_wildcard_for_single_variants`, `fn_params_excessive_bools`,
  `needless_for_each`, `string_add`, `implicit_clone`, `return_self_not_must_use` and
  `allow_attributes_without_reason` as warnings (errors in CI under `-D warnings`).
- `clippy.toml` lets tests unwrap, expect and panic, and disallows `std::panic::catch_unwind`.
- `rustfmt.toml` pins the 100-column width.
- A lint that is wrong for one item is silenced with `#[expect(..., reason = "...")]` on that
  item; `#[allow]` is kept for lints that only sometimes fire, and must carry a `reason`
  (`allow_attributes_without_reason`).

Where the guide conflicts with Oxikube's architecture, the architecture wins and `docs/STYLE.md`
says so next to the rule: ports stay object-safe `dyn` traits shared through `Arc` (ADR 0002),
async is used everywhere (the guide's no-async rule is Chromium-only), and the library-API
rules are recommendations because every crate is application code.

## Consequences

- The existing code was brought in line in the same change, so CI stays green.
- Rules clippy cannot check are review items: one module per `use` with no nested paths,
  abbreviations, enums over bools, comment quality. The PR checklist points at `docs/STYLE.md`.
- The guide's rustfmt options `imports_granularity = "Module"`, `hex_literal_case = "Lower"`
  and `format_macro_matchers = true` need nightly rustfmt. Stable rustfmt warns on every run if
  they are set, so they are left out and the rules are enforced in review. A later change could
  run `cargo +nightly fmt` in CI with those options; it would reformat many files at once.
- `must_use_candidate`, `doc_markdown` and `missing_errors_doc` were measured (hundreds of hits
  each, mostly noise) and are not enabled.

## Alternatives

- Rust's default clippy groups only: the ecosystem baseline, but silent on `unwrap`, assertion
  messages, `SAFETY` comments and glob imports, which are the rules teams disagreed on.
- `clippy::pedantic` wholesale: thousands of warnings, many of them taste, which the guide
  itself argues against ("rules should pull their weight").
