# Rust style

Oxikube follows the [Google Rust Style Guide](https://google.github.io/styleguide/rustguide.html)
(ADR 0018), which itself defers to the [Rust Style Guide](https://doc.rust-lang.org/style-guide/)
and the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/). This page states the
rules as they apply here: where Oxikube's architecture overrides the guide, the override is
written next to the rule. The Chromium, Android and Fuchsia-only rules (`[chrome]`, gtest, no
async, no threads) do not apply.

Every workspace crate is application code (`publish = false`), so the guide's library-API rules
are strong recommendations, not requirements. Prefer the simple shape.

Rules marked **(lint)** are enforced by `[workspace.lints.clippy]` in the root `Cargo.toml`
and `clippy.toml`, and fail CI under `-D warnings`. The rest are enforced in review.

## Code organisation

- Name a leaf module or file after its primary item (`mod foo` / `foo.rs`). A non-leaf module
  (`mod.rs`) holds module declarations, re-exports and the module docs, with little or no logic.
- Keep files small: one abstraction per file is the target, and a file that keeps growing is
  split into a module directory. Crates follow the layer map in `docs/ARCHITECTURE.md`.
- No glob imports **(lint: `wildcard_imports`, `enum_glob_use`)**. Allowed: `use super::*` in a
  `#[cfg(test)]` module, `use gpui::prelude::*` (and any other `prelude`), and `pub use
  submodule::*` in a non-leaf module. Do not define new preludes.
- One module per `use` statement, with no `::` inside `{}`:
  `use std::collections::HashMap; use std::sync::Arc;`, or `use std::{fmt, io};`, but not
  `use std::{collections::HashMap, sync::Arc};`. (This is rustfmt's `imports_granularity =
  "Module"`, which is nightly-only, so reviewers enforce it.)
- Import types and traits unqualified (`HashMap`, `Arc`). Call free functions through their
  module (`mem::take`, `fmt::write`). Qualify or rename a type when the bare name is ambiguous
  (`io::Error`, `use std::fmt::Result as FmtResult`).

## Formatting

- `cargo fmt` is the formatter (`rustfmt.toml`, 100 columns). CI runs `cargo fmt --all --check`.
- The guide also asks for `hex_literal_case = "Lower"` and `format_macro_matchers = true`;
  both are nightly-only, so write lowercase hex (`0xff`) and keep macro matchers tidy by hand.
- Write invisible or confusable characters as `\u{...}` escapes in literals.
- `#[rustfmt::skip]` only for deliberate alignment (tables, matrices) or generated code, never
  for taste.

## Ownership and borrowing

- Prefer unique ownership and borrows to `Rc`/`Arc` and `.clone()`. Clone only for a real
  copy, or when borrowing would make the code much harder to follow. Cloning an `Arc` is fine.
  **Oxikube:** ports are shared as `Arc<dyn Port>` and app services as `Arc<Service>` across
  async tasks and GPUI entities. That is the architecture, not a violation; the rule is about
  data you could own. (`redundant_clone`, `implicit_clone` are lints.)
- Take `&str`, `&[T]`, `Option<&T>` and `&T`, not `&String`, `&Vec<T>`, `&Option<T>` or
  `&Box<T>` **(lint: `ptr_arg`, `borrowed_box`)**.

## Function inputs and outputs

- Returning `Vec<T>` is fine in application code; return an iterator when callers would
  otherwise collect, filter and collect again. Accept `&[T]` or `impl IntoIterator` as the
  caller needs.
- `impl Into<T>` parameters are almost never needed in app code; take the concrete type.
- Use an `enum` instead of an unclear `bool` parameter (`Follow::Tail`, not `true`)
  **(lint: `fn_params_excessive_bools`)**. If a `bool` stays, name it at the call site with an
  inline comment: `open(path, /* create= */ true)`.

## Errors and panics

- `Option` for "no value" that needs no explanation; `Result` for recoverable failures the
  caller needs to understand. Ports and services return `OxiResult<T>` (`docs/ARCHITECTURE.md`,
  "Error taxonomy and mapping guidelines").
- `panic!` only for bugs: a violated invariant, never a cluster, network, file or user-input
  condition. A panic in the UI takes down the app.
- No `.unwrap()` outside tests **(lint: `unwrap_used`)**. When a panic is the right answer, use
  `.expect("...")` and make the message say why the value must be there
  (`.expect("the window was opened above")`), not restate the call.
- Document the panic: a public function that can panic has a `# Panics` rustdoc section
  **(lint: `missing_panics_doc`)**.
- Give `assert!`/`assert_eq!` a message that explains the invariant or logs the values
  (`assert!(rows <= cap, "table grew past its cap: {rows} > {cap}")`)
  **(lint: `missing_assert_message`, outside tests)**.
- Prefer `if let` / `let ... else` / `match` to checking and then unwrapping.
- No `catch_unwind`: `std::panic::catch_unwind` is linted **(lint: `disallowed_methods`)**,
  futures' `FutureExt::catch_unwind` is checked in review. The exceptions are defence in depth
  that nothing relies on (the SQLite worker keeps one bad job from killing the store), tests that
  exercise panic handling, and `oxikube_runtime`'s deterministic test backend for `spawn_kube`,
  which reports a panic as `KubeTaskError::Panicked` the way tokio's `JoinError` does. The std
  call sites carry `#[expect(clippy::disallowed_methods, reason = ...)]`.
- Tests may `unwrap`, `expect` and `panic` freely (`clippy.toml`).
- Never reach inside a poisoned mutex.

## Types

- A newtype when values differ in meaning, need their own API or must not be mixed up
  (`ContextName`, `Namespace`). Use the vocabulary in `docs/CONTEXT.md`.
- Named fields over tuples, except single-field newtypes and very local values.

## Traits

- Use `#[derive]` whenever it gives the right impl.
- Implement standard traits instead of look-alike methods: `FromStr` not `fn from_str`,
  `Display` not `fn to_string`, `Default` alongside `fn new()` with no arguments
  **(lint: `should_implement_trait`, `inherent_to_string`, `new_without_default`)**.
- If a type has `len()`, give it `is_empty()` **(lint: `len_without_is_empty`)**.
- Add `Clone`, `Debug`, `PartialEq`, `Default`, `Copy` when they are cheap and meaningful.
  `Copy` only when the type will stay `Copy`. `PartialEq`/`PartialOrd` unless `Eq`/`Ord` are
  needed (map keys need `Eq + Hash`, sorted keys `Ord`). Do not derive `Clone` on wrappers of
  external resources (connections, child processes).
- Options structs with all-public fields implement `Default` so callers write
  `..Default::default()`.
- Define a trait only for generic code, `dyn`/`impl Trait` boundaries or extension methods,
  never just to group functions. **Oxikube:** ports are object-safe `async_trait` traits and are
  used as `dyn` (ADR 0002); generics stay the default inside domain logic.
- Extension traits end in `Ext` and add methods only.

## Naming

- Follow std's conversions: `as_` (borrowed, free), `to_` (expensive or owned from borrowed),
  `into_` (consumes `self`) **(lint: `wrong_self_convention`)**.
- Constructors: `new()`, `new_with_foo()`, `try_new()` for a fallible one, `from_foo()` paired
  with a `From` impl for value conversions.
- Accessors: `foo()` returns `&T` (or `T` if `Copy`), `foo_mut()`, `set_foo(T)`, `take_foo()`,
  `replace_foo(T)`, `with_foo(|f| ...)`.
- Lifetimes get descriptive names (`'cluster`), except `'_` and the one or two `'a`/`'b` that
  just tie an input to an output. Generic parameters are short and hint at their bound
  (`R: ResourcePort`), `T`/`U` when unbounded.
- No abbreviations a newcomer would not know and never drop letters inside a word (`message`,
  not `msg`; `namespace`, not `ns`). Common ones stay (`id`, `url`, `json`, `tls`, `ip`, and
  Kubernetes' own: `pod`, `pvc`, `crd`, `rbac`, `gvk`).

## Documentation

- Public items have rustdoc; `oxikube_domain` and `oxikube_ports` `#![deny(missing_docs)]`.
- Say something the name does not: why, when, what it costs, what it returns on the edge
  cases. Do not restate the function name. Self-explanatory enum variants need no doc.
- Full sentences with capitals and full stops, Markdown, `///` on items, `//!` at the top of a
  module or crate, `//` for implementation comments and `TODO(#issue)`.
- Document panics (`# Panics`) and the invariants an `unsafe fn` relies on (`# Safety`).

## Other features

- `unsafe` only where no safe alternative exists (FFI, platform APIs), wrapped in a safe
  function, every block preceded by a `// SAFETY:` comment saying why it is sound and holding
  one unsafe operation **(lint: `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`;
  `unsafe_op_in_unsafe_fn` is denied)**. Justify it in the PR; for performance, with numbers.
- No unstable features (the toolchain is stable, `rust-toolchain.toml`).
- No code before `main` (`ctor`-style static initialisers).
- `#[must_use]` on builder methods that return `Self` **(lint: `return_self_not_must_use`)**, on
  pure functions whose result is the whole point, and on lazy types. Not needed on `Result`.
- `#[non_exhaustive]` on public enums and public-field structs that other teams' crates match
  on or build, unless the set is fixed by definition.
- `const fn` only when it will stay `const`.
- Operator overloading only for the operator's ordinary mathematical or logical meaning.
- Macros last: plain Rust, then `macro_rules!`, then a build step, then a proc macro.

## Implementation

- Annotate types where a reader cannot infer them at a glance, especially for `.into()`,
  `.parse()`, `.collect()`. Prefer `let x: Vec<_> = ...` to a turbofish.
- Match every variant where feasible; use `_` only for genuine don't-cares, and name the single
  remaining variant instead of `_` **(lint: `match_wildcard_for_single_variants`)**.
- Keep iterator chains short. Bind a chain to a variable before a `for` loop, and use a `for`
  loop rather than `.for_each` **(lint: `needless_for_each`)**.
- Build strings with `format!`/`write!`/`push_str`, not `+` or `str::replace` on templates
  **(lint: `string_add`)**.
- When a lint is wrong for one item, silence it on that item with
  `#[expect(clippy::<lint>, reason = "...")]`, which fails once the code no longer needs it.
  `#[allow]` is for a lint that may or may not fire (a crate-level `print_stdout` in a bench, a
  test helper module only some test binaries use), and every `#[allow]` carries a `reason`
  **(lint: `allow_attributes_without_reason`)**.
