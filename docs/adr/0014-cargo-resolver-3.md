# ADR 0014: Cargo feature resolver 3

- **Status:** Accepted (2026-10-04)
- **Deciders:** project owner
- **Related:** docs/PLAN.md (workspace layout), `rust-toolchain.toml`

## Context

The workspace manifest set `resolver = "2"`. A virtual workspace does not inherit the
resolver from the edition, so it stays on 2 even though every crate is edition 2024, whose
default is resolver 3. Resolver 3 (stable since Rust 1.84) is the newest; Cargo 1.99 rejects
any higher value.

## Decision

Set `resolver = "3"` in the root `Cargo.toml`.

Resolver 3 keeps the feature unification rules of resolver 2 (build, dev and target-specific
features stay separate) and adds MSRV-aware dependency resolution: when `cargo update` or a new
dependency picks versions, it prefers releases whose `rust-version` the pinned toolchain
(`rust-toolchain.toml`) can build, instead of picking a newer one that fails to compile.

## Consequences

- No lockfile change today: existing resolved versions already build on 1.99.
- Future `cargo update` and dependabot bumps avoid releases that need a newer compiler than
  the pinned toolchain; bumping the toolchain is what unlocks them.
- Features unify exactly as before, so build times and binary contents do not change.

## Alternatives

- Stay on resolver 2: works, but loses MSRV-aware resolution and leaves the workspace behind
  the edition default.
