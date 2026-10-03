# oxikube (binary)

**Layer:** `bins`

The application binary. It wires adapters into `oxikube_app`, mounts the UI crates, and owns the
init order (Zed `main.rs` pattern): logging → settings → keymap → theme → `AppState` global →
each crate's `init(cx)` → workspace restore. It is the only crate allowed to depend on every
layer.

Today it opens a placeholder window that proves the GPUI stack (`gpui-pre` + `gpui-component`)
builds and renders. E05 replaces this with the real shell, and E05-S13 holds it to the 400 ms
startup budget (`docs/PERFORMANCE.md`).

## Allowed internal dependencies

- everything (domain, ports, app, adapters, platform, ui, testing)

`gpui-component` is declared here only to prove the dependency stack resolves; once E05-S02
lands, views reach it through `oxikube_ui` only.
