# .gpui-overlay (generated)

This directory holds the GPUI crates Oxikube patches, each one the exact pinned crate from
crates.io with the diffs in `patches/gpui/<crate>-<version>/` applied (ADR 0017). The workspace
`[patch.crates-io]` builds them from here. Only this README is committed.

If cargo says it `failed to read .gpui-overlay/<crate>-<version>/Cargo.toml`, the overlay has
not been built in this checkout (or a patch changed). Run:

```
scripts/gpui-overlay.sh
```

It needs bash, tar, git and curl (or the crate in cargo's download cache), and is a no-op when
nothing changed. `cargo xtask setup` installs git hooks that run it; CI runs it before cargo.
Never edit files here: put the change in a patch (see `patches/gpui/README.md`).
