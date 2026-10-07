# oxikube_logs_ui

**Layer:** `ui`

Log viewer (single/aggregate/JSON), search, export, send-to-agent.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_workspace`
- `oxikube_keymap`, `oxikube_runtime`, `oxikube_settings` (platform)

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Modules

- `settings`, `follow` (E08-S01, S10): the `logs` settings (buffer, default tail, wrap, timestamps, JSON detect; per-cluster overrides) and their hot reload.
- `view` (E08-S02): `LogView`, a pod's log as a workspace tab.
- `commands` (E08-S02): `pod::ViewLogs` and `logs::*` on the bus, and `LogViews`, which opens and drives the views.
- `row_actions` (E08-S02): "View Logs" on pod rows.
- `settings`, `runtime`, `follow` (E08-S01): `logs.buffer_lines` and the service's runtime.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
