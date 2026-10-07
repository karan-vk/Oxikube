//! The `terminal` settings: which shell a local terminal runs, how it looks (font, cursor, bell)
//! and how much scrollback every terminal keeps.
//!
//! ```json
//! "terminal": {
//!   "shell": null, "shell_args": [],
//!   "font_family": null, "font_size": null, "line_height": 1.3,
//!   "cursor_shape": "block", "cursor_blink": false, "bell": "visual",
//!   "scrollback_lines": 10000, "copy_on_select": false,
//!   "option_as_meta": null, "confirm_multiline_paste": true
//! }
//! ```
//!
//! Layers merge `default.json` -> the user's `settings.json` -> `clusters.<id>.terminal`. A cluster's
//! block matters for what is decided per terminal when it starts, `shell` and `shell_args`
//! ([`TerminalSettings::for_cluster`]); the look and the input settings belong to the window and
//! are read from the top-level block. Out-of-range numbers are clamped (with a warning in the log
//! naming the setting, never the value).
//!
//! How a change reaches what is open (E09-S11):
//!
//! | Setting | Effect of a change |
//! |---|---|
//! | `shell`, `shell_args` | terminals opened afterwards only; a running shell is never replaced |
//! | `font_family`, `font_size`, `line_height` | every open terminal re-lays out once: the element reads them on its next frame and the shaped-row cache is rebuilt once |
//! | `cursor_shape`, `cursor_blink` | the grid's default cursor changes at once (a cursor the process set stays) |
//! | `bell` | read when the next bell rings |
//! | `scrollback_lines` | the grid's history is trimmed or allowed to grow at once |
//! | `copy_on_select`, `option_as_meta`, `confirm_multiline_paste` | the next selection, keystroke or paste |
//!
//! Scrollback is never written to disk, whatever `scrollback_lines` says (non-negotiable 5).

mod content;
mod resolved;
#[cfg(test)]
mod tests;

pub use content::{BellSetting, CursorShapeSetting, TerminalContent};
pub use resolved::{
    MAX_FONT_SIZE, MAX_LINE_HEIGHT, MIN_FONT_SIZE, MIN_LINE_HEIGHT, TerminalSettings,
};
