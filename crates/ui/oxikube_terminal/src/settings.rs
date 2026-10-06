//! The `terminal` settings: which shell a local terminal runs, and how much scrollback every
//! terminal keeps (E09-S11 adds font and cursor).
//!
//! ```json
//! "terminal": { "shell": null, "shell_args": [], "scrollback_lines": 10000 }
//! ```
//!
//! `scrollback_lines` is how much history each terminal keeps in memory, capped at
//! [`MAX_SCROLLBACK_LINES`]. Scrollback is never written to disk. A change applies at once to
//! every open terminal ([`crate::TerminalState`] observes the setting).

use oxikube_settings::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::grid::{DEFAULT_SCROLLBACK_LINES, MAX_SCROLLBACK_LINES};

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
        }
    }
}

oxikube_settings::register_settings!(TerminalSettings);

impl TerminalSettings {
    /// The global value, or the defaults when no settings store exists (tests, previews).
    pub fn current(cx: &gpui::App) -> Self {
        Self::try_get(cx).cloned().unwrap_or_default()
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
        let content: TerminalContent = serde_json::from_str(r#"{"scrollback_lines": 500}"#).unwrap();
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
    }
}
