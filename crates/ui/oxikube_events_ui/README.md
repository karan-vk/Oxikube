# oxikube_events_ui

**Layer:** `ui`

Cluster-wide events view and per-resource events tab.

- `settings` (E14-S02): `events.buffer`, followed into the app's `EventService`.
- `view`, `model`, `filter`, `persist`, `export`, `commands` (E14-S03): the events view. `:ev`, the sidebar's Events heading
  or the palette's "Show Events" (`events::Open`) open it in the cluster's tab. See the crate docs.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- `oxikube_app`
- `oxikube_settings` (the `events` settings)
- `oxikube_keymap` (the `EventsView` key context)
- `oxikube_runtime` (`notify_coalesced`, `spawn_kube`)
- `oxikube_ui`
- `oxikube_workspace`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
