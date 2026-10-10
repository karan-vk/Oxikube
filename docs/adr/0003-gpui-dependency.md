# ADR 0003: GPUI via exact-pinned gpui-pre snapshots aligned with gpui-component

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/
- **Amended by:** ADR 0017 (GPUI fixes newer than the snapshot as a patch overlay on the exact pinned crates)

## Context

crates.io `gpui` 0.2.2 is frozen (Oct 2025); Zed's main split GPUI into gpui/gpui_platform/gpui_macos/… and publishes rarely. The community `gpui-pre-*` crates (published weekly by the gpui-component maintainer as snapshots of a named Zed commit, Apache-2.0) are what gpui-component 0.7.0 pins exactly (=0.3.7 ↔ zed@1a28cff). A Zed git dependency would be the same code but ~500 MB clones and incompatible with gpui-component (two different `gpui` crates). Zed's own GPL crates above GPUI drag 90–154 internal crates (workspace, editor, agent_ui) and are not reusable as libraries.

## Decision

Depend on `gpui = { package = "gpui-pre", version = "=0.3.7" }` + gpui-pre-platform/macros, and gpui-component/gpui-base/gpui-kit-assets `=0.7.0`, all exact. Bump every snapshot crate together in one dedicated PR. `cargo xtask check-gpui-pin` verifies exactness and the known pairing table (xtask/src/check_gpui_pin/mod.rs). Documented exit: switch to a Zed git dependency at the commit named by the snapshot plus a forked gpui-kit pointing at the same rev.

### Pairing table

The authoritative table. `PAIRS` in `xtask/src/check_gpui_pin/mod.rs` must match it row for row; a GPUI bump PR edits both in the same change and re-runs `cargo deny check` (the RUSTSEC ignores in `deny.toml` are re-justified on every bump).

| gpui-pre / gpui-pre-platform / gpui-pre-macros | gpui-component / gpui-base / gpui-kit-assets | Zed commit named by the snapshot |
|---|---|---|
| `=0.3.7` | `=0.7.0` | `1a28cff` |

## Consequences

Weekly API churn is absorbed on our schedule, not upstream's. Single third-party publisher risk is mitigated by committed Cargo.lock, optional vendoring, and the git-rev fallback. GPUI fixes newer than the snapshot require waiting for the next snapshot, or a patch in the overlay of ADR 0017 (`patches/gpui/`), dropped at the bump that ships the fix.
