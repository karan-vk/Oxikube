//! The per-cluster interactive exec policy, as the connect view words it.

use oxikube_ports::ExecInteractivity;

/// What the cluster's `exec_interactivity` setting means to the user: whether a credential
/// plugin (`aws eks get-token`, `kubelogin`, an MFA helper) may stop to ask something.
///
/// The setting is E06-S08's; this only names it for the screen: `never` is *forbid*,
/// `if_available` is *ask* (the plugin may use a terminal when one exists) and `always` is
/// *allow*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecPolicy {
    /// The setting's value.
    pub setting: ExecInteractivity,
}

impl ExecPolicy {
    /// The policy a session was opened with.
    pub fn of(setting: ExecInteractivity) -> Self {
        Self { setting }
    }

    /// The one-word name: Forbid, Ask or Allow.
    pub fn label(self) -> &'static str {
        match self.setting {
            ExecInteractivity::Never => "Forbid",
            ExecInteractivity::IfAvailable => "Ask",
            ExecInteractivity::Always => "Allow",
        }
    }

    /// The setting's value as written in `settings.json` (`exec_interactivity`).
    pub fn setting_value(self) -> &'static str {
        match self.setting {
            ExecInteractivity::Never => "never",
            ExecInteractivity::IfAvailable => "if_available",
            ExecInteractivity::Always => "always",
        }
    }

    /// What the policy means for a plugin that wants to prompt.
    pub fn explanation(self) -> &'static str {
        match self.setting {
            ExecInteractivity::Never => {
                "Credential plugins run without a terminal and may not prompt."
            }
            ExecInteractivity::IfAvailable => {
                "Credential plugins may prompt when a terminal is available."
            }
            ExecInteractivity::Always => "Credential plugins may stop and prompt for input.",
        }
    }

    /// Whether a plugin may be interactive at all.
    pub(super) fn allows_interaction(self) -> bool {
        self.setting != ExecInteractivity::Never
    }

    /// What to do to authenticate, given the policy and whether Oxikube has a terminal to offer.
    pub fn instructions(self, terminal: bool) -> &'static str {
        match (self.allows_interaction(), terminal) {
            (false, true) => {
                "Interactive sign-in is forbidden for this cluster. Open a terminal, sign in with \
                 your provider's tool (the login you use for kubectl), then retry."
            }
            (false, false) => {
                "Interactive sign-in is forbidden for this cluster. Sign in outside Oxikube, in a \
                 terminal, with your provider's tool (the login you use for kubectl), then retry."
            }
            (true, true) => {
                "Sign in again, then retry. Open a terminal to run your provider's login, or \
                 retry and let the plugin prompt."
            }
            (true, false) => {
                "Sign in again with your provider's tool in a terminal, then retry, or retry and \
                 let the plugin prompt."
            }
        }
    }
}
