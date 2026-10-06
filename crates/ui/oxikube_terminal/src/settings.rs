//! The `terminal` settings: which shell a local terminal runs.

use oxikube_settings::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
}

/// The resolved `terminal` settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalSettings {
    /// The configured shell; `None` means "decide from `SHELL`" (see
    /// [`resolve_shell`](crate::backend::local::resolve_shell)).
    pub shell: Option<String>,
    /// Arguments for the shell.
    pub shell_args: Vec<String>,
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
        }
    }
}

oxikube_settings::register_settings!(TerminalSettings);

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
        };
        let settings = TerminalSettings::from_content(content);
        assert_eq!(settings.shell, None);
        assert_eq!(settings.shell_args, ["-l"]);
        let named = TerminalContent {
            shell: Some(" /bin/zsh ".into()),
            shell_args: None,
        };
        assert_eq!(
            TerminalSettings::from_content(named).shell.as_deref(),
            Some("/bin/zsh")
        );
    }
}
