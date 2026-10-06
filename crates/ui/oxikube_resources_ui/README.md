# oxikube_resources_ui

**Layer:** `ui`

Generic resource table + detail drawer, per-kind panels and actions, create/bulk ops, CRD browsing, apply UI, file browser, audit viewer.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- platform crates (`oxikube_runtime`, `oxikube_keymap`, `oxikube_theme`, ...)
- `oxikube_workspace`
- `oxikube_palette`
- `oxikube_editor`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Modules

- `table` (E07-S03): `ResourceTable`, the generic virtualised table of one kind (store
  subscription, sort through the store, column layout per kind, multi-select, keyboard, context
  menu, status colours from the theme's `oxikube` block).
- `views` (E07-S03): `ResourceViews`, which opens tables in cluster tabs from the sidebar and
  `resource::OpenList`, and runs `resource::Open`, `CopyName` and `SelectAll`.

Bench: `cargo run -p oxikube_resources_ui --profile release-fast --example table_bench`.
Screenshot: `cargo test -p oxikube_resources_ui --features screenshot --test screenshot`.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
