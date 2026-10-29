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

## The audit log (E19-S08)

`audit_log` is append-only: `record` (the `AuditRecord` JSON, no bodies) is the source of truth and
`verb`, `initiator`, `outcome`, `batch_id`, `kind`, `namespace` and `name` are virtual generated
columns over it (migration 0002), so they need no backfill. Indices, newest first:
`(ts_ns, id)`, `(cluster, ts_ns, id)`, `(initiator, ts_ns, id)`, `(verb, ts_ns, id)` and a partial
`(batch_id, id)`; `query_audit` builds its `WHERE` from the filters that are set so each can use
its index, and pages by keyset on `(ts_ns, id)`. A trigger refuses every `UPDATE` and every
`DELETE` except one inside `prune_audit`, which parks its cutoff in `audit_prune` and removes only
older rows, then gives the pages back (`incremental_vacuum`; a database from before 0002 is
converted to incremental auto-vacuum by its first prune).
