# xtask

**Layer:** `bins` (repository automation; not shipped)

Run as `cargo xtask <command>` (aliases `xtask` and `x` in `.cargo/config.toml`).

| Command | What it does |
|---|---|
| `lint-deps` | Enforces the hexagonal dependency direction and the banned-crate list over `cargo metadata --no-deps` (docs/ARCHITECTURE.md, ADR 0002). The check is a pure function over `cargo_metadata::Metadata`, unit-tested against fixture JSON in `xtask/tests/fixtures/`. |
| `check-gpui-pin` | Verifies the `gpui-pre*` and `gpui-component`/`gpui-base`/`gpui-kit-assets` pins are exact `=` and a known pair from the table in ADR 0003; prints the Zed commit of the snapshot. |
| `kind-up` / `kind-down` | Local kind cluster for integration tests. |
| `load-pods` | Pause-pod load generator (`--count`, `--namespaces`, `--churn`, `--cleanup`, `--context`) for performance work; refuses non-`kind-*` contexts unless `--allow-non-kind`. See docs/PERFORMANCE.md. |

## Allowed internal dependencies

- everything (it only shells out and parses metadata today; `tokio` is used for Ctrl-C handling in `load-pods`)

## Tests

`cargo test -p xtask`. When a layer rule in ARCHITECTURE.md changes, update `Layer` in
`src/lint_deps.rs`, the fixtures and the tests together.
