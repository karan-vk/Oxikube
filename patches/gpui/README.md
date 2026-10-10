# GPUI patches (the overlay of ADR 0017)

Oxikube builds a few `gpui-pre` crates with fixes that upstream does not have yet. Each fix is a
unified diff against the **exact pinned crate from crates.io**; `scripts/gpui-overlay.sh` builds
`.gpui-overlay/<crate>-<version>/` (gitignored) from the `.crate` plus these diffs, and the
workspace `[patch.crates-io]` builds the crate from there. No GPUI source is committed.

```
scripts/gpui-overlay.sh            # build what is missing or stale (no-op otherwise)
scripts/gpui-overlay.sh --check    # verify: overlay == pinned crate + patches
cargo xtask check-gpui-pin         # pins, patch directories, [patch.crates-io], then --check
```

## The patches

| Patch | Why | Upstream status |
|---|---|---|
| `gpui-pre-macos-0.3.7/0001-overlay-allow-warnings.patch` | The overlay builds the crate as a path crate, whose lints cargo does not cap; `#![allow(warnings)]` keeps it building under CI's `RUSTFLAGS=-D warnings` as the registry crate does. | Not for upstream (overlay only). Goes with the crate's last fix. |
| `gpui-pre-macos-0.3.7/0002-draw-late-resize-in-its-transaction.patch` | Live resize: the frame of a new window size waited for the next display pass even when that pass was a refresh late, so a resize swept every refresh missed refreshes (E05-P601, #601: 53 to 129 per 10 s in `tabs-panes` `resize-window`). `set_frame_size` now draws it at once, in the resize's own Core Animation transaction, when a refresh or more has passed since the last frame. | Zed PR draft "gpui_macos: Draw a resized window at once when its display pass is late" (to file). |
| `gpui-pre-apple-0.3.7/0001-overlay-allow-warnings.patch` | As for `gpui-pre-macos`. | Not for upstream (overlay only). |
| `gpui-pre-apple-0.3.7/0002-prefetch-next-drawable.patch` | `-[CAMetalLayer nextDrawable]` blocked the main thread in `MetalRenderer::draw` until the window server released a drawable (5 to 10 ms when it came late or a resize reallocated them), so the window missed refreshes. A worker thread now fetches the next drawable as soon as a frame is presented, overlapping the wait with the next frame's layout and paint (E05-P601, #601). | Zed PR draft "gpui_apple: Prefetch the next CAMetalLayer drawable on a worker thread" (to file). |

## Adding a patch

1. Extract the pinned crate (`scripts/gpui-overlay.sh` already did: copy
   `.gpui-overlay/<crate>-<version>/` elsewhere, `git init` it and commit), make the change there,
   and `git diff > patches/gpui/<crate>-<version>/NNNN-<slug>.patch` (paths `a/src/...`, relative
   to the crate root). Keep it minimal: one fix per patch.
2. Start the file with a header (any text before the first `diff --git`): what it fixes, the
   crate and its Zed path, `Upstream:` (the Zed issue or PR draft) and the licence line
   `Patch: Copyright Oxikube contributors, GPL-3.0-or-later`. The crates themselves are
   Apache-2.0 (Copyright Zed Industries, Inc. and contributors); see `THIRD_PARTY_NOTICES.md`.
3. A crate patched for the first time also gets `0001-overlay-allow-warnings.patch` (copy one
   above), `checksum` (its `checksum = "..."` line from Cargo.lock before patching, which is
   also the crates.io index's `cksum`), and `<crate> = { path = ".gpui-overlay/<crate>-<version>" }`
   under `[patch.crates-io]`.
4. Add a row above, run `scripts/gpui-overlay.sh` and `cargo xtask check-gpui-pin`.

Never edit `.gpui-overlay/` by hand: `--check` fails on any file that is not the pinned crate
plus its patches.

## At a GPUI pin bump

For each patch: if the new snapshot contains the fix, delete the patch (and the crate's
directory and `[patch.crates-io]` entry when no fix is left); otherwise rebase it onto the new
version, rename the directory, update `checksum` and the `[patch.crates-io]` path. Then
`scripts/gpui-overlay.sh && cargo xtask check-gpui-pin`.
