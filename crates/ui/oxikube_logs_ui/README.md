# oxikube_logs_ui

**Layer:** `ui`

Log viewer (single/aggregate/JSON), search, export, send-to-agent.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_workspace`
- `oxikube_keymap`, `oxikube_runtime`, `oxikube_settings`, `oxikube_theme` (platform)

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Modules

- `settings`, `follow` (E08-S01, S10, S04, S07): the `logs` settings (buffer, default tail, wrap, timestamps, JSON detect, max streams, reconnect retries; per-cluster overrides) and their hot reload.
- `view` (E08-S02): `LogView`, a pod's log as a workspace tab.
- `view::recovery` (E08-S07): after the stream stopped, "Follow replacement" (`logs::FollowReplacement`, `shift-r`) and "Reconnect" (`logs::Reconnect`, `r`).
- `view::aggregate` (E08-S04): `LogView::workload`, a workload's or Service's pods merged: pod gutters and colours, the banner, the Sources menu.
- `commands` (E08-S02, E08-S04): `pod::ViewLogs`, `workload::ViewLogs` and `logs::*` on the bus, and `LogViews`, which opens and drives the views.
- `search` (E08-S03): the `/` bar: regex with case and inverse toggles, highlight or filter mode, next / previous match with a count; the match index is `oxikube_app::logs::MatchIndex`.
- `row_actions` (E08-S02, E08-S04): "View Logs" on pod rows, and on workload and Service rows.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
