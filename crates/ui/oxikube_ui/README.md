# oxikube_ui

**Layer:** `ui`

Thin wrapper over gpui-component: tokens, curated components (Table, DockArea, Dialog, Menu, Input, Tabs, Sidebar, Charts, Markdown, Editor glue), icons, zoom-safe sizes. The ONLY crate allowed to import gpui_component.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_theme`
- `oxikube_assets`
- `oxikube_settings`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
