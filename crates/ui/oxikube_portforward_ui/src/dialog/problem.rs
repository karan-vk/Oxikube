//! [`StartProblem`]: a failed start as the dialog shows it, with the one-click fix.

use std::io::ErrorKind;

use oxikube_app::StartError;
use oxikube_domain::OxiError;
use oxikube_ports::PortForwardError;

use super::form::FIRST_UNPRIVILEGED_PORT;

/// What one click can change to try again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    /// Let Oxikube pick the local port.
    UseAuto,
    /// Bind the loopback address.
    UseLoopback,
}

impl Fix {
    /// The button's text.
    pub fn label(self) -> &'static str {
        match self {
            Self::UseAuto => "Use Auto",
            Self::UseLoopback => "Bind 127.0.0.1",
        }
    }
}

/// A start that failed: what went wrong, what to do, and a fix to apply with one click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartProblem {
    /// What went wrong, in the user's terms.
    pub message: String,
    /// What to do about it.
    pub suggestion: Option<String>,
    /// The change that retries with a better chance.
    pub fix: Option<Fix>,
}

impl StartProblem {
    /// The problem a failed `portforward::Start` shows. Reads the typed [`StartError`] the
    /// command's error carries; any other error shows its own message.
    pub fn of(error: &OxiError) -> Self {
        match StartError::of(error) {
            Some(typed) => Self::of_start(typed),
            None => Self {
                message: error.message().to_owned(),
                suggestion: None,
                fix: None,
            },
        }
    }

    /// The problem for a [`StartError`].
    pub fn of_start(error: &StartError) -> Self {
        match error {
            StartError::Forward(forward) => Self::of_forward(forward),
            StartError::ReadOnly(_) => Self {
                message: error.to_string(),
                suggestion: Some(
                    "Lift read-only mode, or allow port-forwards on read-only clusters \
                     (portforward.allowed_in_readonly)."
                        .to_owned(),
                ),
                fix: None,
            },
            StartError::NotConnected(_) => Self {
                message: error.to_string(),
                suggestion: Some("Connect to the cluster, then try again.".to_owned()),
                fix: None,
            },
            other => Self {
                message: other.to_string(),
                suggestion: None,
                fix: None,
            },
        }
    }

    /// The problem for a [`PortForwardError`]: the story's messages and fixes.
    pub fn of_forward(error: &PortForwardError) -> Self {
        let message = error.to_string();
        let (suggestion, fix) = match error {
            PortForwardError::BindFailed { addr, kind } => match kind {
                ErrorKind::AddrInUse => (
                    Some(format!(
                        "Port {} is in use. Use Auto or choose another local port.",
                        addr.port()
                    )),
                    Some(Fix::UseAuto),
                ),
                ErrorKind::PermissionDenied if addr.port() < FIRST_UNPRIVILEGED_PORT => (
                    Some(format!(
                        "Ports below {FIRST_UNPRIVILEGED_PORT} need privileges. Use a port \
                         \u{2265} {FIRST_UNPRIVILEGED_PORT}, or Auto."
                    )),
                    Some(Fix::UseAuto),
                ),
                ErrorKind::AddrNotAvailable => (
                    Some("That address does not belong to this machine.".to_owned()),
                    Some(Fix::UseLoopback),
                ),
                _ => (error.suggestion(), Some(Fix::UseAuto)),
            },
            PortForwardError::Forbidden(_) => (
                Some("You need `create` on `pods/portforward` in this namespace.".to_owned()),
                None,
            ),
            PortForwardError::PodNotReady(_) => (
                Some("The pod is not ready yet. Wait for it to be running, then try again.".into()),
                None,
            ),
            other => (other.suggestion(), None),
        };
        Self {
            message,
            suggestion,
            fix,
        }
    }
}
