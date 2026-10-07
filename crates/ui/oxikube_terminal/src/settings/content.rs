//! What one settings layer says about terminals: the `terminal` object of `settings.json`
//! ([`TerminalContent`]) and the choices it offers ([`CursorShapeSetting`], [`BellSetting`]).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `terminal.cursor_shape`: the cursor a terminal shows until its process asks for another one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CursorShapeSetting {
    /// A filled block (the default).
    #[default]
    Block,
    /// A thin vertical bar.
    Bar,
    /// A line under the character.
    Underline,
}

/// `terminal.bell`: what a bell (`BEL`, `^G`) from the process does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BellSetting {
    /// Nothing.
    None,
    /// The terminal flashes briefly (the default).
    #[default]
    Visual,
    /// The system's alert sound.
    Audible,
}

/// What one settings layer says about terminals: the `terminal` object of `settings.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct TerminalContent {
    /// The shell a local terminal runs (a path or a name found on `PATH`). `null` uses the
    /// `SHELL` environment variable, then `/bin/sh`. Applies to terminals opened afterwards;
    /// `clusters.<id>.terminal.shell` sets it for one cluster's terminals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Arguments passed to the shell, e.g. `["-l"]` for a login shell. Empty by default: a
    /// login shell re-reads the profile files and slows opening a terminal, so it is only
    /// started when you ask for it here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_args: Option<Vec<String>>,
    /// Font family of the terminal text; should be monospace. `null` uses the platform's
    /// monospace font (Menlo, Consolas, DejaVu Sans Mono). Applies to open terminals at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// Font size in points (6 - 72); a value outside is clamped. `null` uses the theme's
    /// monospace size. The UI zoom applies on top. Applies to open terminals at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 6.0, max = 72.0))]
    pub font_size: Option<f32>,
    /// Row height as a multiple of the font size (1.0 - 2.5); a value outside is clamped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1.0, max = 2.5))]
    pub line_height: Option<f32>,
    /// Lines of history each terminal keeps in memory (0 - 100000); a larger value is clamped.
    /// Never saved to disk, whatever this says. Lowering it drops the oldest lines of open
    /// terminals at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(max = 100_000))]
    pub scrollback_lines: Option<usize>,
    /// Copy the selection to the clipboard as soon as the mouse button is released (default
    /// `false`). `cmd-c` (`ctrl-shift-c` elsewhere) always copies the selection explicitly, and
    /// `ctrl-c` always reaches the process as an interrupt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_on_select: Option<bool>,
    /// The cursor shape until the process sets its own (`block`, `bar` or `underline`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_shape: Option<CursorShapeSetting>,
    /// Whether that cursor blinks (default `false`). A process that asks for a blinking or a
    /// steady cursor is obeyed until it resets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_blink: Option<bool>,
    /// What a bell from the process does: `none`, `visual` (a brief flash, the default) or
    /// `audible` (the system alert sound).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bell: Option<BellSetting>,
    /// Whether Alt (Option) sends `ESC` before the key, the "meta" prefix shells and editors use
    /// for `alt-b`, `alt-f`, `alt-.`. `null` (the default) is `false` on macOS, where Option
    /// types characters (`å`, `∫`), and `true` elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_as_meta: Option<bool>,
    /// Ask before pasting text with more than one line (default `true`): a pasted newline runs
    /// the line. Single-line pastes never ask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_multiline_paste: Option<bool>,
    /// The shells `pod::Shell` tries in a container, first to last, each probed with a quick
    /// exec; the first one the container has is opened (default `["bash", "sh"]`, like Lens).
    /// A name or an absolute path. An empty list uses the default. Applies to shells opened
    /// afterwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_shells: Option<Vec<String>>,
}
