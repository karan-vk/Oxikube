# ADR 0017: GPUI fixes as a patch overlay on the exact pinned gpui-pre crates

- **Status:** Accepted (2026-10-10)
- **Deciders:** project owner (decision on #601, 2026-10-10), story E05-P601 (#601)
- **Amends:** ADR 0003 (GPUI via exact-pinned gpui-pre snapshots). The pins, the pairing table and
  "bumped together" stand; what changes is that a pinned crate may carry documented patches.
- **Related:** `patches/gpui/README.md` (the patches and their upstream status), ADR 0016 (the
  zero-jank budget the first patches serve), `THIRD_PARTY_NOTICES.md`

## Context

ADR 0003 takes GPUI fixes newer than the snapshot only through the next snapshot ("or vendoring").
E05-P601 found the dropped refreshes of a live window resize and of the drawable wait inside
`gpui-pre-macos` / `gpui-pre-apple` 0.3.7, which Oxikube code cannot reach, and the newest snapshot
(0.3.8) has not changed those paths. Waiting means a Zed PR, a gpui-pre snapshot and a gpui-component
release pinned to it. A git dependency on Zed is excluded by ADR 0003; a vendored copy of the crates
in the repository would put about 15 000 lines of GPUI source under review that nobody wrote here,
and would drift silently from the pin.

## Decision

1. **Patch files only.** A GPUI fix is a unified diff against the exact pinned crate from crates.io,
   committed as `patches/gpui/<crate>-<version>/NNNN-<slug>.patch`, applied in order. Each patch
   starts with a header naming the crate, the Zed path, the upstream issue or PR draft, and its
   licence (the patch is GPL-3.0-or-later; the crate is Apache-2.0). The directory also holds
   `checksum`, the sha256 of the `.crate` that Cargo.lock recorded before the crate was patched
   (a patched crate's lock entry loses its checksum). `patches/gpui/README.md` lists every patch,
   why it exists and its upstream status. No GPUI source is committed.
2. **The overlay.** `scripts/gpui-overlay.sh` (bash, tar, git, curl; no cargo, because cargo cannot
   resolve the workspace while the overlay is missing) takes the `.crate` from cargo's download
   cache or `static.crates.io`, checks its sha256, extracts it into the gitignored
   `.gpui-overlay/<crate>-<version>/`, applies the patches with `git apply` (a patch that does not
   apply fails the script and removes the stale overlay, so nothing builds from it), and writes a
   stamp of the crate checksum and patch hashes, so a second run is a no-op. `--check` rebuilds each
   overlay in a scratch directory and compares it with `.gpui-overlay/`.
3. **Wiring.** `[patch.crates-io]` points each patched crate at `.gpui-overlay/<crate>-<version>`,
   and `[workspace] exclude` keeps those path crates out of the members. The first patch of each
   crate (`0001-overlay-allow-warnings`) allows warnings at the crate root: cargo caps the lints of
   registry crates but not of path crates, and CI builds with `RUSTFLAGS=-D warnings`. CI runs the
   script in `.github/actions/setup-rust` (every job that builds Rust, before any cargo command),
   `cargo xtask setup` runs it, and the pre-commit and pre-push hooks run it first.
4. **Verification.** `cargo xtask check-gpui-pin` checks, besides the pins: every patch directory
   matches the pinned `gpui-pre` version and Cargo.lock, has its exact `[patch.crates-io]` path
   entry, a checksum, headed patches listed in the README, and that no GPUI crate is patched from
   any other source; then it runs `scripts/gpui-overlay.sh --check`, which proves that the patches
   apply cleanly and that the overlay is the pinned crate plus the patches and nothing else.
5. **Upstream and bumps.** Each fix is offered to Zed. At a GPUI pin bump, a patch whose fix
   shipped is deleted; the others are rebased onto the new version (directory renamed with its
   `[patch.crates-io]` path and `checksum`). The bump PR keeps doing what ADR 0003 asks.
6. Only crates that need a fix are patched, with the smallest diff that fixes it.

## Consequences

- A fresh checkout runs `scripts/gpui-overlay.sh` once before cargo (cargo's own error names the
  missing `.gpui-overlay/...` path; `.gpui-overlay/README.md` and the comment above
  `[patch.crates-io]` say what to run). Every worktree builds its own overlay.
- The patched crates are path crates: they build incrementally like workspace code, and a change to
  a patch rebuilds them and their dependents.
- The overlay's provenance is checkable: crate checksum + patch files, both in the repository.
- Patches are a liability at every pin bump; the README's upstream column says which ones to expect
  to drop.

## Alternatives

- **Wait for upstream** (ADR 0003 as written): weeks to months per fix; the budget of ADR 0016
  stays failed meanwhile.
- **Zed git dependency at a fork:** excluded by ADR 0003 (clone size, two `gpui` crates with
  gpui-component).
- **Vendored copy of the crates in the repository:** reviewable only as a diff against the pin
  anyway, and it drifts without a check. The owner chose the overlay over the copy.
