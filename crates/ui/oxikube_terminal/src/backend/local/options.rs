//! What a local terminal runs and with which environment.

use std::path::PathBuf;

use oxikube_ports::exec::TerminalSize;
use portable_pty::CommandBuilder;

use super::kubeconfig::ClusterEnv;
use crate::settings::TerminalSettings;

/// The size a terminal starts with until the element reports its real one.
pub const DEFAULT_SIZE: TerminalSize = TerminalSize {
    width: 80,
    height: 24,
    pixel_width: 0,
    pixel_height: 0,
};

/// How to start a [`LocalPty`](super::LocalPty).
#[derive(Debug, Clone)]
pub struct LocalPtyOptions {
    /// The shell (`terminal.shell`); `None` resolves like [`resolve_shell`] with no setting.
    pub shell: Option<String>,
    /// Arguments for the shell (`terminal.shell_args`).
    pub args: Vec<String>,
    /// The working directory; `None` leaves it to the PTY layer (the user's home).
    pub cwd: Option<PathBuf>,
    /// The initial size.
    pub size: TerminalSize,
    /// The cluster the terminal belongs to: its kubeconfig, context and namespace go into the
    /// shell's environment. `None` for a plain shell.
    pub cluster: Option<ClusterEnv>,
}

impl Default for LocalPtyOptions {
    fn default() -> Self {
        Self {
            shell: None,
            args: Vec::new(),
            cwd: None,
            size: DEFAULT_SIZE,
            cluster: None,
        }
    }
}

impl LocalPtyOptions {
    /// Options for the shell the `terminal` settings name.
    pub fn from_settings(settings: &TerminalSettings) -> Self {
        Self {
            shell: settings.shell.clone(),
            args: settings.shell_args.clone(),
            ..Self::default()
        }
    }

    /// Starts the shell in `cwd`.
    #[must_use]
    pub fn in_dir(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Opens the terminal for `cluster`.
    #[must_use]
    pub fn for_cluster(mut self, cluster: ClusterEnv) -> Self {
        self.cluster = Some(cluster);
        self
    }

    /// Starts at `size`.
    #[must_use]
    pub fn with_size(mut self, size: TerminalSize) -> Self {
        self.size = size;
        self
    }

    /// The program to run: the configured shell, else `$SHELL`, else `/bin/sh`.
    pub fn resolved_shell(&self) -> String {
        resolve_shell(
            self.shell.as_deref(),
            std::env::var("SHELL").ok().as_deref(),
        )
    }
}

/// Picks the shell: the setting when it is non-blank, else the `SHELL` variable when
/// non-blank, else `/bin/sh` (`cmd.exe` on Windows).
pub fn resolve_shell(setting: Option<&str>, env_shell: Option<&str>) -> String {
    [setting, env_shell]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|shell| !shell.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "cmd.exe".to_owned()
            } else {
                "/bin/sh".to_owned()
            }
        })
}

/// The command to spawn: the shell and its arguments. The environment is inherited as it is
/// (`PATH` included); only `TERM`, `COLORTERM` and the cluster variables are set.
pub(super) fn build_command(
    options: &LocalPtyOptions,
    cluster_vars: &[(&'static str, String)],
) -> CommandBuilder {
    let mut command = CommandBuilder::new(options.resolved_shell());
    command.args(&options.args);
    if let Some(cwd) = &options.cwd {
        command.cwd(cwd);
    }
    // The grid speaks xterm; a GUI launch has no TERM at all.
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    for (key, value) in cluster_vars {
        command.env(key, value);
    }
    command
}
