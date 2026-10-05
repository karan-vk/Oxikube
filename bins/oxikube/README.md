# oxikube (binary)

**Layer:** `bins`

The application binary. It wires adapters into `oxikube_app`, mounts the UI crates, and owns the
init order (Zed `main.rs` pattern): logging → settings → keymap → theme → `AppState` global →
each crate's `init(cx)` → workspace restore. It is the only crate allowed to depend on every
layer.

Today it opens the themed main window (`oxikube_workspace::window`: `Root`, title bar, application
menu; E05-S03) with an empty body; E05-S04 mounts the workspace in it and E05-S13 holds startup to
the 400 ms budget (`docs/PERFORMANCE.md`). Linux packaging assets (`.desktop` file, icon) live in
`resources/linux/` and are named after `oxikube_workspace::window::APP_ID`.

## Flags

`oxikube --help` lists them. `--perf` (with `--perf-duration`, `--perf-dir`) records frame times,
feed throughput, notify counts and resident memory (RSS) to `<data dir>/oxikube/perf/*.jsonl` and prints p50/p95/p99 on
exit. `--perf-scenario <name>` (feature `perf-scenarios`, never in default or release builds) runs
one headless perf sample; `cargo xtask perf` drives it. See docs/PERFORMANCE.md ("Perf harness").

## Allowed internal dependencies

- everything (domain, ports, app, adapters, platform, ui, testing)

`gpui-component` is declared here only to prove the dependency stack resolves; once E05-S02
lands, views reach it through `oxikube_ui` only.
