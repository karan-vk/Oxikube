//! The `terminal` settings: which shell a local terminal runs, and how much scrollback every
//! terminal keeps (E09-S11 adds font and cursor).
//!
//! ```json
//! "terminal": {
//!   "shell": null, "shell_args": [], "scrollback_lines": 10000,
//!   "copy_on_select": false, "option_as_meta": null, "confirm_multiline_paste": true
//! }
//! ```
//!
//! The input settings (E09-S06) take effect on the next keystroke, selection or paste: nothing
//! caches them.
//!
//! `scrollback_lines` is how much history each terminal keeps in memory, capped at
//! [`MAX_SCROLLBACK_LINES`]. Scrollback is never written to disk. A change applies at once to
//! every open terminal ([`crate::TerminalState`] observes the setting).

use oxikube_settings::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::grid::{DEFAULT_SCROLLBACK_LINES, MAX_SCROLLBACK_LINES};

/// `terminal.option_as_meta` when the setting is `null`: off on macOS (Option composes
/// characters), on elsewhere.
const DEFAULT_OPTION_AS_META: bool = !cfg!(target_os = "macos");
/// `terminal.copy_on_select` default.
const DEFAULT_COPY_ON_SELECT: bool = false;
/// `terminal.confirm_multiline_paste` default.
const DEFAULT_CONFIRM_MULTILINE_PASTE: bool = true;

/// What one settings layer says about terminals: the `terminal` object of `settings.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct TerminalContent {
    /// The shell a local terminal runs (a path or a name found on `PATH`). `null` uses the
    /// `SHELL` environment variable, then `/bin/sh`. Applies to terminals opened afterwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Arguments passed to the shell, e.g. `["-l"]` for a login shell. Empty by default: a
    /// login shell re-reads the profile files and slows opening a terminal, so it is only
    /// started when you ask for it here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_args: Option<Vec<String>>,
    /// Lines of history each terminal keeps in memory (0 - 100000). Never saved to disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(max = 100_000))]
    pub scrollback_lines: Option<usize>,
    /// Copy the selection to the clipboard as soon as the mouse button is released (default
    /// `false`). `cmd-c` (`ctrl-shift-c` elsewhere) always copies the selection explicitly, and
    /// `ctrl-c` always reaches the process as an interrupt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_on_select: Option<bool>,
    /// Whether Alt (Option) sends `ESC` before the key, the "meta" prefix shells and editors use
    /// for `alt-b`, `alt-f`, `alt-.`. `null` (the default) is `false` on macOS, where Option
    /// types characters (`å`, `∫`), and `true` elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_as_meta: Option<bool>,
    /// Ask before pasting text with more than one line (default `true`): a pasted newline runs
    /// the line. Single-line pastes never ask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_multiline_paste: Option<bool>,
}

/// The resolved `terminal` settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSettings {
    /// The configured shell; `None` means "decide from `SHELL`" (see
    /// [`resolve_shell`](crate::backend::local::resolve_shell)).
    pub shell: Option<String>,
    /// Arguments for the shell.
    pub shell_args: Vec<String>,
    /// Lines of history per terminal, at most [`MAX_SCROLLBACK_LINES`].
    pub scrollback_lines: usize,
    /// Copy the selection when the mouse button is released.
    pub copy_on_select: bool,
    /// Alt sends `ESC` + the key (resolved: the platform default when the setting is `null`).
    pub option_as_meta: bool,
    /// Ask before pasting several lines.
    pub confirm_multiline_paste: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self::from_content(TerminalContent::default())
    }
}

impl Settings for TerminalSettings {
    const KEY: Option<&'static str> = Some("terminal");
    type Content = TerminalContent;

    fn from_content(content: TerminalContent) -> Self {
        Self {
            shell: content
                .shell
                .map(|shell| shell.trim().to_owned())
                .filter(|shell| !shell.is_empty()),
            shell_args: content.shell_args.unwrap_or_default(),
            scrollback_lines: content
                .scrollback_lines
                .unwrap_or(DEFAULT_SCROLLBACK_LINES)
                .min(MAX_SCROLLBACK_LINES),
            copy_on_select: content.copy_on_select.unwrap_or(DEFAULT_COPY_ON_SELECT),
            option_as_meta: content.option_as_meta.unwrap_or(DEFAULT_OPTION_AS_META),
            confirm_multiline_paste: content
                .confirm_multiline_paste
                .unwrap_or(DEFAULT_CONFIRM_MULTILINE_PASTE),
        }
    }
}

oxikube_settings::register_settings!(TerminalSettings);

impl TerminalSettings {
    /// The global value, or the defaults when no settings store exists (tests, previews).
    pub fn current(cx: &gpui::App) -> Self {
        Self::try_get(cx).cloned().unwrap_or_default()
    }

    /// `option_as_meta` without cloning the settings: read on every keystroke.
    pub fn option_as_meta(cx: &gpui::App) -> bool {
        Self::try_get(cx).map_or(DEFAULT_OPTION_AS_META, |settings| settings.option_as_meta)
    }

    /// `copy_on_select` without cloning the settings.
    pub fn copy_on_select(cx: &gpui::App) -> bool {
        Self::try_get(cx).map_or(DEFAULT_COPY_ON_SELECT, |settings| settings.copy_on_select)
    }

    /// `confirm_multiline_paste` without cloning the settings.
    pub fn confirm_multiline_paste(cx: &gpui::App) -> bool {
        Self::try_get(cx).map_or(DEFAULT_CONFIRM_MULTILINE_PASTE, |settings| {
            settings.confirm_multiline_paste
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_leave_the_choice_to_the_environment() {
        let settings = TerminalSettings::from_content(TerminalContent::default());
        assert_eq!(settings, TerminalSettings::default());
        assert!(settings.shell.is_none() && settings.shell_args.is_empty());
    }

    #[test]
    fn a_blank_shell_counts_as_unset() {
        let content = TerminalContent {
            shell: Some("  ".into()),
            shell_args: Some(vec!["-l".into()]),
            ..TerminalContent::default()
        };
        let settings = TerminalSettings::from_content(content);
        assert_eq!(settings.shell, None);
        assert_eq!(settings.shell_args, ["-l"]);
        let named = TerminalContent {
            shell: Some(" /bin/zsh ".into()),
            ..TerminalContent::default()
        };
        assert_eq!(
            TerminalSettings::from_content(named).shell.as_deref(),
            Some("/bin/zsh")
        );
    }

    #[test]
    fn defaults_to_ten_thousand_lines_and_caps_the_value() {
        assert_eq!(TerminalSettings::default().scrollback_lines, 10_000);
        let content: TerminalContent =
            serde_json::from_str(r#"{"scrollback_lines": 500}"#).unwrap();
        assert_eq!(
            TerminalSettings::from_content(content).scrollback_lines,
            500
        );
        let content: TerminalContent =
            serde_json::from_str(r#"{"scrollback_lines": 5000000}"#).unwrap();
        assert_eq!(
            TerminalSettings::from_content(content).scrollback_lines,
            MAX_SCROLLBACK_LINES
        );
    }

    #[test]
    fn the_shipped_defaults_match() {
        let store =
            oxikube_settings::SettingsStore::new(oxikube_assets::default_settings()).unwrap();
        let settings: &TerminalSettings = store.get(None);
        assert_eq!(settings.scrollback_lines, DEFAULT_SCROLLBACK_LINES);
        assert_eq!(settings.shell, None);
        assert!(settings.shell_args.is_empty());
        assert!(!settings.copy_on_select);
        assert!(settings.confirm_multiline_paste);
        assert_eq!(settings, &TerminalSettings::default());
    }

    #[test]
    fn the_input_settings_default_and_override() {
        let settings = TerminalSettings::default();
        assert!(!settings.copy_on_select);
        assert!(settings.confirm_multiline_paste);
        // Option composes characters on macOS, so meta is off there and on elsewhere.
        assert_eq!(settings.option_as_meta, !cfg!(target_os = "macos"));
        let content: TerminalContent = serde_json::from_str(
            r#"{"copy_on_select": true, "option_as_meta": true, "confirm_multiline_paste": false}"#,
        )
        .unwrap();
        let settings = TerminalSettings::from_content(content);
        assert!(settings.copy_on_select && settings.option_as_meta);
        assert!(!settings.confirm_multiline_paste);
        let off: TerminalContent = serde_json::from_str(r#"{"option_as_meta": false}"#).unwrap();
        assert!(!TerminalSettings::from_content(off).option_as_meta);
    }
}
