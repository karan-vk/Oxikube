# oxikube_overview_ui

**Layer:** `ui`

Cluster overview dashboard, charts, pulses.

- `pulses` (E13-S10): the pulses page. `:pulses`, the palette's "Show Pulses" (`cluster::ShowPulses`) or the sidebar's Metrics > Pulses open it in the cluster's tab. See the module docs of `src/pulses/mod.rs`.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_ui`
- `oxikube_workspace`
- `oxikube_keymap`, `oxikube_runtime` (platform)

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
