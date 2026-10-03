# ADR 0010: Local state in SQLite via rusqlite behind StatePort

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/

## Context

Window layout, open tabs, recent clusters, favourites, per-cluster namespace choice, port-forward favourites, audit log, caches and agent threads need durable, queryable storage with partial updates. JSON files get messy for this; settings stay human-editable JSON separately.

## Decision

`oxikube_state_sqlite` implements `StatePort` on rusqlite (bundled) with in-repo migrations. Settings/keymap/themes remain JSON files under the config dir. Secrets never go to SQLite; they go to the OS keychain via `SecretStorePort`.

## Consequences

One dependency with a C build; corruption fallback must reset state without losing settings. Zed uses the same split (db crate + settings.json).
