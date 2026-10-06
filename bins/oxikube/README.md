# oxikube (binary)

**Layer:** `bins`

The application binary. It wires adapters into `oxikube_app`, mounts the UI crates, and owns the
init order (Zed `main.rs` pattern), documented stage by stage in `src/startup/mod.rs` and summarised
in `docs/ARCHITECTURE.md` ("App start-up and init order"): logging and the panic hook → assets →
runtime → settings → theme → keymap → ui → state db (background open) → `AppState` → workspace →
feature crates → keymap re-bind → open window. It is the only crate allowed to depend on every
layer.

The crate is a library (`src/lib.rs`: `startup`, `app_state`, `kube_ports`, `mount`, `cluster_prefs`)
plus the thin `main.rs`, so tests can run the real init order. `AppState::test(cx)` (tests, or feature `test-support`) runs it on testkit
fakes. To add a feature crate's `init(cx)`, add a line to `startup::FEATURES`.

Files the app writes live in the data directory (`$OXIKUBE_DATA_DIR`, else
`~/Library/Application Support/oxikube` on macOS, `~/.local/share/oxikube` on Linux): `logs/`
(daily rolling `oxikube.<date>.log`, seven kept, secrets redacted), `crashes/` (a redacted report per
panic, newest twenty kept; nothing is uploaded) and `state.db`. Settings, keymap and themes are in the
config directory (`$OXIKUBE_CONFIG_DIR`, else `~/.config/oxikube`).

It opens the themed main window (`oxikube_workspace::window`: `Root`, title bar, application menu;
E05-S03) and mounts the cluster UI in it before the first frame (`src/mount`, E07-S00): the catalog
home as the first tab, the hotbar, the cluster tabs (each with its sidebar, connect views and
namespace selector), the status bar item, session restore, and the command bus with its
`MutationGuard` (stored in `AppState`). The cluster side runs on `oxikube_kube` (`src/kube_ports`:
the kubeconfig catalog, built on its first use after the first frame, and the connector);
E05-S13 holds startup to the 400 ms budget (`docs/PERFORMANCE.md`).

Using it: `cargo run -p oxikube`, pick a context in the catalog (type to search, Enter or click)
and its cluster tab opens with the sidebar; "Kubeconfig sources" in the catalog header opens the
sources screen. `cargo test -p oxikube --features integration --test kind_app` (with
`OXIKUBE_TEST_CONTEXT=kind-oxikube`) runs the same path against the kind cluster. Linux packaging assets (`.desktop` file, icon) live in
`resources/linux/` and are named after `oxikube_workspace::window::APP_ID`.

## Flags

`oxikube --help` lists them. `--perf` (with `--perf-duration`, `--perf-dir`) records frame times,
feed throughput, notify counts and resident memory (RSS) to `<data dir>/oxikube/perf/*.jsonl` and prints p50/p95/p99 on
exit. `--perf-scenario <name>` (feature `perf-scenarios`, never in default or release builds) runs
one headless perf sample; `cargo xtask perf` drives it. See docs/PERFORMANCE.md ("Perf harness").

Two hidden tooling flags (not in `--help`; they print and exit before logging or any window) make
this binary the generator behind `cargo xtask gen-settings-schema`: `--print-settings-schema` prints
`settings.schema.json` and `--print-settings-crates` lists the crates that registered settings. The
binary links every settings-owning crate, so the schema covers every setting the app accepts
(`src/settings_schema.rs`, E05-S06b). A feature crate that registers settings must be a dependency
of this binary and be referenced from it (its `init` in `startup::FEATURES`); xtask fails otherwise.

## Allowed internal dependencies

- everything (domain, ports, app, adapters, platform, ui, testing)

`gpui-component` is declared here only to prove the dependency stack resolves; once E05-S02
lands, views reach it through `oxikube_ui` only.
