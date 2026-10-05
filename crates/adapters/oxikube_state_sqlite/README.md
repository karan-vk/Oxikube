# oxikube_state_sqlite

**Layer:** `adapters`

StatePort adapter on rusqlite (bundled): migrations, workspace layout, per-cluster state, favourites, audit log, caches.

## What it does

`SqliteState::open(path)` opens (creating) the database on its own thread, runs the embedded
migrations, and implements `StatePort` (kv, typed tables, audit log). A damaged file is moved to
`<name>.corrupt-<timestamp>` and replaced by a fresh database; `SqliteState::recovery()` reports
it. Tests use a temp directory. Open-time numbers: `cargo run -p oxikube_state_sqlite --release --example open_bench`.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
