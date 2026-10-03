//! Interactivity policy for exec credential plugins.
//!
//! kubeconfig `exec` users (`aws eks get-token`, `gke-gcloud-auth-plugin`, `kubelogin`,
//! ...) may declare `interactiveMode: Never | IfAvailable | Always`. kube-rs lets a plugin
//! inherit the parent's stdin/stderr unless the mode is `Never`, which is the right thing
//! for a terminal client and wrong for a GUI: nobody can answer a prompt that is written
//! to a console no one is looking at, and a plugin blocked on stdin would hang the
//! connection. [`ExecInteractivePolicy`] is the ceiling the host allows; the default is
//! [`Never`](ExecInteractivePolicy::Never).
//!
//! Under `IfAvailable` and `Always` kube lets the plugin inherit stderr, so a failed run
//! carries no stderr text and the prompt detection in the `classify` module has
//! nothing to read: such failures are reported as a generic, retryable plugin failure. Only
//! `Never` (the default) captures stderr and can explain why a plugin needs a person.
//!
//! Applying the policy never loosens a plugin's own setting. A plugin that *requires*
//! interaction (`Always`) under a stricter policy is rejected up front with
//! [`ErrorKind::Auth`](oxikube_domain::ErrorKind::Auth) so the session moves to
//! `AuthRequired` with an explanation instead of failing later in an opaque way.
//!
//! Building a client runs the plugin synchronously (`Client::try_from` calls it), so
//! [`build_client`] does that on the blocking pool under a deadline, never on the UI thread.

use std::path::Path;
use std::time::Duration;

use kube::config::{ExecConfig, ExecInteractiveMode};
use kube::{Client, Config};
use oxikube_domain::{OxiError, OxiResult};

use super::classify::{CredentialRefresh, classify_with};

/// How interactive an exec credential plugin is allowed to be.
///
/// Ordered by permissiveness: `Never < IfAvailable < Always`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ExecInteractivePolicy {
    /// Plugins run with no stdin and may not prompt (the GUI default).
    #[default]
    Never,
    /// Plugins may use a terminal when one exists. kube-rs then inherits stdin/stderr.
    IfAvailable,
    /// Plugins that demand interaction are allowed (a terminal host, tests).
    Always,
}

impl ExecInteractivePolicy {
    /// Rewrites `exec.interactive_mode` so the plugin cannot exceed this policy.
    ///
    /// * `Never`: `IfAvailable` (and an unset mode, which client-go treats as
    ///   `IfAvailable`) is downgraded to `Never`; `Always` is an error.
    /// * `IfAvailable`: `Never` and `IfAvailable` are kept; `Always` is an error.
    /// * `Always`: nothing changes.
    ///
    /// # Errors
    ///
    /// A non-retryable `Auth` error naming the plugin when it requires interaction the
    /// policy does not allow.
    pub fn apply_to_exec(self, exec: &mut ExecConfig) -> OxiResult<()> {
        let requested = match exec.interactive_mode {
            Some(ExecInteractiveMode::Never) => Self::Never,
            Some(ExecInteractiveMode::IfAvailable) | None => Self::IfAvailable,
            Some(ExecInteractiveMode::Always) => Self::Always,
        };
        if self == Self::Always {
            return Ok(());
        }
        if requested == Self::Always {
            return Err(OxiError::auth(
                format!(
                    "the exec credential plugin{} requires interactive input (interactiveMode: Always), which Oxikube cannot provide. Sign in with the plugin in a terminal, or set interactiveMode to IfAvailable or Never in the kubeconfig",
                    plugin_label(exec)
                ),
                false,
            ));
        }
        let effective = requested.min(self);
        exec.interactive_mode = Some(match effective {
            Self::Never => ExecInteractiveMode::Never,
            Self::IfAvailable | Self::Always => ExecInteractiveMode::IfAvailable,
        });
        Ok(())
    }

    /// Applies the policy to the exec plugin of `config`, if it has one.
    ///
    /// # Errors
    ///
    /// See [`apply_to_exec`](Self::apply_to_exec).
    pub fn apply_to_config(self, config: &mut Config) -> OxiResult<()> {
        match config.auth_info.exec.as_mut() {
            Some(exec) => self.apply_to_exec(exec),
            None => Ok(()),
        }
    }
}

/// Applies `policy`, then builds a [`Client`] without blocking the caller or waiting forever.
///
/// kube runs exec credential plugins synchronously inside `Client::try_from` with no timeout
/// of its own, so the build runs on the blocking pool under `deadline`. A plugin that does
/// not finish in time (a device-code poll, a browser login, a read on `/dev/tty`) gives a
/// retryable `Timeout` error and the session can move on.
///
/// Limits: on timeout the blocking thread, and the plugin child process, may keep running
/// after this returns (Rust cannot cancel either). A plugin that hangs later, when kube
/// refreshes an expiring token inside a live client, is not covered by this deadline.
///
/// # Errors
///
/// The policy's `Auth` error, the classified build failure, or `Timeout`.
pub async fn build_client(
    mut config: Config,
    policy: ExecInteractivePolicy,
    deadline: Duration,
) -> OxiResult<Client> {
    policy.apply_to_config(&mut config)?;
    let refresh = CredentialRefresh::of(&config.auth_info);
    let build = tokio::task::spawn_blocking(move || Client::try_from(config));
    match tokio::time::timeout(deadline, build).await {
        Ok(Ok(Ok(client))) => Ok(client),
        Ok(Ok(Err(err))) => Err(classify_with(&err, refresh)),
        Ok(Err(join)) => Err(OxiError::internal(format!(
            "building the client failed unexpectedly: {join}"
        ))),
        Err(_elapsed) => Err(OxiError::timeout(format!(
            "exec credential plugin did not finish within {}",
            describe(deadline)
        ))),
    }
}

fn describe(d: Duration) -> String {
    if d.as_secs() >= 1 {
        format!("{}s", d.as_secs())
    } else {
        format!("{}ms", d.as_millis())
    }
}

/// ` `name``: the plugin's executable name only (never its arguments or environment).
fn plugin_label(exec: &ExecConfig) -> String {
    exec.command
        .as_deref()
        .and_then(|c| Path::new(c).file_name())
        .map(|n| format!(" `{}`", n.to_string_lossy()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_domain::ErrorKind;

    fn exec(mode: Option<ExecInteractiveMode>) -> ExecConfig {
        ExecConfig {
            command: Some("/usr/local/bin/aws".into()),
            args: Some(vec!["--secret-arg".into()]),
            interactive_mode: mode,
            ..Default::default()
        }
    }

    #[test]
    fn default_policy_is_never() {
        assert_eq!(
            ExecInteractivePolicy::default(),
            ExecInteractivePolicy::Never
        );
    }

    #[test]
    fn never_downgrades_if_available_and_unset() {
        for mode in [
            None,
            Some(ExecInteractiveMode::IfAvailable),
            Some(ExecInteractiveMode::Never),
        ] {
            let mut e = exec(mode);
            ExecInteractivePolicy::Never.apply_to_exec(&mut e).unwrap();
            assert_eq!(e.interactive_mode, Some(ExecInteractiveMode::Never));
        }
    }

    #[test]
    fn never_rejects_always_with_explanatory_auth_error() {
        let mut e = exec(Some(ExecInteractiveMode::Always));
        let err = ExecInteractivePolicy::Never
            .apply_to_exec(&mut e)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Auth);
        assert!(!err.is_retryable());
        assert!(err.message().contains("`aws`"), "{}", err.message());
        assert!(err.message().contains("interactive"));
        assert!(!err.message().contains("secret-arg"));
        // The config is left as it was.
        assert_eq!(e.interactive_mode, Some(ExecInteractiveMode::Always));
    }

    #[test]
    fn if_available_policy_keeps_modes_and_rejects_always() {
        let p = ExecInteractivePolicy::IfAvailable;
        let mut e = exec(Some(ExecInteractiveMode::Never));
        p.apply_to_exec(&mut e).unwrap();
        assert_eq!(e.interactive_mode, Some(ExecInteractiveMode::Never));
        let mut e = exec(None);
        p.apply_to_exec(&mut e).unwrap();
        assert_eq!(e.interactive_mode, Some(ExecInteractiveMode::IfAvailable));
        assert!(
            p.apply_to_exec(&mut exec(Some(ExecInteractiveMode::Always)))
                .is_err()
        );
    }

    #[test]
    fn always_policy_changes_nothing() {
        let mut e = exec(None);
        ExecInteractivePolicy::Always.apply_to_exec(&mut e).unwrap();
        assert_eq!(e.interactive_mode, None);
        let mut e = exec(Some(ExecInteractiveMode::Always));
        ExecInteractivePolicy::Always.apply_to_exec(&mut e).unwrap();
        assert_eq!(e.interactive_mode, Some(ExecInteractiveMode::Always));
    }

    #[test]
    fn config_without_exec_is_untouched() {
        let mut config = Config::new("https://127.0.0.1:6443".parse().unwrap());
        ExecInteractivePolicy::Never
            .apply_to_config(&mut config)
            .unwrap();
        assert!(config.auth_info.exec.is_none());
    }

    #[test]
    fn applies_to_the_config_exec() {
        let mut config = Config::new("https://127.0.0.1:6443".parse().unwrap());
        config.auth_info.exec = Some(exec(None));
        ExecInteractivePolicy::Never
            .apply_to_config(&mut config)
            .unwrap();
        assert_eq!(
            config.auth_info.exec.unwrap().interactive_mode,
            Some(ExecInteractiveMode::Never)
        );
    }
}
