# oxikube_resources_ui

**Layer:** `ui`

Generic resource table + detail drawer, per-kind panels and actions, create/bulk ops, CRD browsing, apply UI, file browser, audit viewer.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_workspace`
- `oxikube_palette`
- `oxikube_editor`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
