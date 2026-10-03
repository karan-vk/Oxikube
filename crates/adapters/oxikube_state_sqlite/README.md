# oxikube_state_sqlite

**Layer:** `adapters`

StatePort adapter on rusqlite (bundled): migrations, workspace layout, per-cluster state, favourites, audit log, caches.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
