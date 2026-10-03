# oxikube_argocd_ui

**Layer:** `ui`

Argo CD + Rollouts views (apps, detail, sync, diff, appsets, projects, settings).

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_workspace`
- `oxikube_editor`
- `oxikube_logs_ui`
- `oxikube_terminal`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
