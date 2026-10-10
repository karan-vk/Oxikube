//! `oxikube_palette` — layer: `ui`.
//!
//! Command palette, ':' jump bar, pickers (vendored Zed PickerDelegate design), help overlay.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`picker`] | E11-S02 | [`Picker`] and [`PickerDelegate`]: a fuzzy query field over a virtualised list of matches, keyboard selection, confirm / secondary confirm, dismiss; presented through the workspace's modal layer. GPL-3.0-or-later code derived from Zed's `picker` crate |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod picker;

pub use picker::{Picker, PickerDelegate, match_label};
