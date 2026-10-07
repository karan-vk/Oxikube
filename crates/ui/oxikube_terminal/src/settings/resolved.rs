//! [`TerminalSettings`]: the resolved `terminal` settings and how a store is read for them.

use gpui::{App, SharedString};
use oxikube_domain::ids::ClusterId;
use oxikube_settings::{Settings, SettingsLocation, SettingsStore};

use super::content::{BellSetting, CursorShapeSetting, TerminalContent};
use crate::element::metrics::DEFAULT_LINE_HEIGHT;
use crate::grid::{CursorShape, DEFAULT_SCROLLBACK_LINES, DefaultCursor, MAX_SCROLLBACK_LINES};

/// The smallest `terminal.font_size`.
pub const MIN_FONT_SIZE: f32 = 6.0;
/// The largest `terminal.font_size`.
pub const MAX_FONT_SIZE: f32 = 72.0;
/// The smallest `terminal.line_height`.
pub const MIN_LINE_HEIGHT: f32 = 1.0;
/// The largest `terminal.line_height`.
pub const MAX_LINE_HEIGHT: f32 = 2.5;

/// `terminal.option_as_meta` when the setting is `null`: off on macOS (Option composes
/// characters), on elsewhere.
const DEFAULT_OPTION_AS_META: bool = !cfg!(target_os = "macos");
/// `terminal.copy_on_select` default.
const DEFAULT_COPY_ON_SELECT: bool = false;
/// `terminal.confirm_multiline_paste` default.
const DEFAULT_CONFIRM_MULTILINE_PASTE: bool = true;
/// `terminal.exec_shells` default: the shell fallback chain of `pod::Shell`.
pub const DEFAULT_EXEC_SHELLS: [&str; 2] = ["bash", "sh"];

/// The resolved `terminal` settings.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalSettings {
    /// The configured shell; `None` means "decide from `SHELL`" (see
    /// [`resolve_shell`](crate::backend::local::resolve_shell)).
    pub shell: Option<String>,
    /// Arguments for the shell.
    pub shell_args: Vec<String>,
    /// The configured font family; `None` is the platform's monospace font.
    pub font_family: Option<SharedString>,
    /// The configured font size in points, within [`MIN_FONT_SIZE`] .. [`MAX_FONT_SIZE`]; `None`
    /// is the theme's monospace size.
    pub font_size: Option<f32>,
    /// Row height as a multiple of the font size, within [`MIN_LINE_HEIGHT`] ..
    /// [`MAX_LINE_HEIGHT`].
    pub line_height: f32,
    /// Lines of history per terminal, at most [`MAX_SCROLLBACK_LINES`].
    pub scrollback_lines: usize,
    /// Copy the selection when the mouse button is released.
    pub copy_on_select: bool,
    /// The cursor shape until the process sets its own.
    pub cursor_shape: CursorShapeSetting,
    /// Whether that cursor blinks.
    pub cursor_blink: bool,
    /// What a bell does.
    pub bell: BellSetting,
    /// Alt sends `ESC` + the key (resolved: the platform default when the setting is `null`).
    pub option_as_meta: bool,
    /// Ask before pasting several lines.
    pub confirm_multiline_paste: bool,
    /// The shell chain of `pod::Shell` (never empty).
    pub exec_shells: Vec<String>,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self::from_content(TerminalContent::default())
    }
}

/// `value` within `min ..= max`; a non-finite value is `None` (the default applies). A value
/// outside is clamped and said so in the log (the setting's name and bounds, never anything
/// else from the file).
fn clamped(name: &str, value: Option<f32>, min: f32, max: f32) -> Option<f32> {
    let value = value.filter(|value| value.is_finite())?;
    if !(min..=max).contains(&value) {
        tracing::warn!(
            setting = name,
            min,
            max,
            "terminal setting out of range: clamped"
        );
    }
    Some(value.clamp(min, max))
}

impl Settings for TerminalSettings {
    const KEY: Option<&'static str> = Some("terminal");
    type Content = TerminalContent;

    fn from_content(content: TerminalContent) -> Self {
        let scrollback_lines = content.scrollback_lines.unwrap_or(DEFAULT_SCROLLBACK_LINES);
        if scrollback_lines > MAX_SCROLLBACK_LINES {
            tracing::warn!(
                setting = "terminal.scrollback_lines",
                max = MAX_SCROLLBACK_LINES,
                "terminal setting out of range: clamped"
            );
        }
        Self {
            shell: non_blank(content.shell),
            shell_args: content.shell_args.unwrap_or_default(),
            font_family: non_blank(content.font_family).map(SharedString::from),
            font_size: clamped(
                "terminal.font_size",
                content.font_size,
                MIN_FONT_SIZE,
                MAX_FONT_SIZE,
            ),
            line_height: clamped(
                "terminal.line_height",
                content.line_height,
                MIN_LINE_HEIGHT,
                MAX_LINE_HEIGHT,
            )
            .unwrap_or(DEFAULT_LINE_HEIGHT),
            scrollback_lines: scrollback_lines.min(MAX_SCROLLBACK_LINES),
            copy_on_select: content.copy_on_select.unwrap_or(DEFAULT_COPY_ON_SELECT),
            cursor_shape: content.cursor_shape.unwrap_or_default(),
            cursor_blink: content.cursor_blink.unwrap_or(false),
            bell: content.bell.unwrap_or_default(),
            option_as_meta: content.option_as_meta.unwrap_or(DEFAULT_OPTION_AS_META),
            confirm_multiline_paste: content
                .confirm_multiline_paste
                .unwrap_or(DEFAULT_CONFIRM_MULTILINE_PASTE),
            exec_shells: exec_shells(content.exec_shells),
        }
    }
}

/// `text` trimmed; `None` when it is absent or blank.
fn non_blank(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// The chain from the setting: names trimmed, blanks and repeats dropped; the default when none
/// is left.
fn exec_shells(configured: Option<Vec<String>>) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    for shell in configured.unwrap_or_default() {
        let shell = shell.trim();
        if !shell.is_empty() && !chain.iter().any(|seen| seen == shell) {
            chain.push(shell.to_owned());
        }
    }
    if chain.is_empty() {
        chain = DEFAULT_EXEC_SHELLS.map(str::to_owned).to_vec();
    }
    chain
}

oxikube_settings::register_settings!(TerminalSettings);

impl TerminalSettings {
    /// The global value, or the defaults when no settings store exists (tests, previews).
    pub fn current(cx: &App) -> Self {
        Self::try_get(cx).cloned().unwrap_or_default()
    }

    /// The value for `cluster`'s terminals: the global value with that cluster's
    /// `clusters.<id>.terminal` overrides on top (the global value for `None`, or without a store).
    pub fn for_cluster(cx: &App, cluster: Option<&ClusterId>) -> Self {
        let Some(store) = cx.try_global::<SettingsStore>() else {
            return Self::default();
        };
        let location = cluster.map(|cluster| SettingsLocation { cluster });
        store.try_get::<Self>(location).cloned().unwrap_or_default()
    }

    /// The cursor the grid shows until the process sets its own.
    pub fn default_cursor(&self) -> DefaultCursor {
        DefaultCursor {
            shape: match self.cursor_shape {
                CursorShapeSetting::Block => CursorShape::Block,
                CursorShapeSetting::Bar => CursorShape::Beam,
                CursorShapeSetting::Underline => CursorShape::Underline,
            },
            blinking: self.cursor_blink,
        }
    }

    /// `option_as_meta` without cloning the settings: read on every keystroke.
    pub fn option_as_meta(cx: &App) -> bool {
        Self::try_get(cx).map_or(DEFAULT_OPTION_AS_META, |settings| settings.option_as_meta)
    }

    /// `copy_on_select` without cloning the settings.
    pub fn copy_on_select(cx: &App) -> bool {
        Self::try_get(cx).map_or(DEFAULT_COPY_ON_SELECT, |settings| settings.copy_on_select)
    }

    /// `confirm_multiline_paste` without cloning the settings.
    pub fn confirm_multiline_paste(cx: &App) -> bool {
        Self::try_get(cx).map_or(DEFAULT_CONFIRM_MULTILINE_PASTE, |settings| {
            settings.confirm_multiline_paste
        })
    }

    /// `bell` without cloning the settings.
    pub fn bell(cx: &App) -> BellSetting {
        Self::try_get(cx).map_or(BellSetting::default(), |settings| settings.bell)
    }
}
