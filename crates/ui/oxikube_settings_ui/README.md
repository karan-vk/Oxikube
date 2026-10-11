# oxikube_settings_ui

**Layer:** `ui`

GUI settings page, keymap editor, theme picker.

## What is here

- `metrics` (E13-S05): the cluster's metrics page. Which Prometheus-compatible service the history graphs come from, what detection found and which services it probed (or `metrics unavailable: <reason>`), the `prometheus` settings in effect, "Look again" and the on/off switch (bus commands). Opened by `cluster::ShowMetricsSettings` (palette, the sidebar's Metrics entry, agents) in the cluster's tab.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_workspace`
- `oxikube_settings`
- `oxikube_keymap`
- `oxikube_theme`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
