//! [`Banner`]: the strip above a terminal's screen when its session dropped, ended or never
//! started: a headline, one line of help and the actions that apply.

use oxikube_ports::ExitStatus;

use super::{Failure, FailureKind, Lifecycle};

/// How serious the banner looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Something ended normally (a shell exited with code 0).
    Info,
    /// The session ended or dropped; the user can start it again.
    Warning,
    /// The session could not start.
    Error,
}

/// What a banner button does. Each is a command on the bus (`terminal::Reconnect`,
/// `terminal::Restart`, `terminal::Close`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerAction {
    /// Open the pod session again (`terminal::Reconnect`).
    Reconnect,
    /// Start a fresh local shell (`terminal::Restart`).
    Restart,
    /// Close the tab (`terminal::Close`).
    CloseTab,
}

impl BannerAction {
    /// The button's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Reconnect => "Reconnect",
            Self::Restart => "Restart",
            Self::CloseTab => "Close tab",
        }
    }
}

/// What a banner says. See the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Banner {
    /// How serious it looks.
    pub tone: Tone,
    /// The first line.
    pub headline: String,
    /// What to do about it (and what the server said), one or two sentences.
    pub detail: String,
    /// The buttons, the primary one first.
    pub actions: Vec<BannerAction>,
}

/// Said under every pod banner: a new session is a new shell.
const SESSION_GONE: &str = "Reconnecting starts a new session: the state of the old shell is gone.";

/// Said under a local shell's exit banner.
const RESTART_HINT: &str = "Restart starts a fresh shell with the same settings.";

impl Banner {
    pub(super) fn of(lifecycle: &Lifecycle, local: bool) -> Option<Self> {
        match lifecycle {
            Lifecycle::Connecting | Lifecycle::Running | Lifecycle::Closed => None,
            Lifecycle::Disconnected(failure) => Some(Self::disconnected(failure)),
            Lifecycle::Exited(status) if local => Some(Self::shell_exited(status)),
            Lifecycle::Exited(status) => Some(Self::session_ended(status)),
            Lifecycle::Failed(failure) => Some(Self::failed(failure, local)),
        }
    }

    fn disconnected(failure: &Failure) -> Self {
        Self {
            tone: Tone::Warning,
            headline: failure.headline().to_owned(),
            detail: with_detail(failure, SESSION_GONE),
            actions: vec![BannerAction::Reconnect],
        }
    }

    fn failed(failure: &Failure, local: bool) -> Self {
        Self {
            tone: Tone::Error,
            headline: failure.headline().to_owned(),
            detail: with_detail(failure, ""),
            actions: vec![if local {
                BannerAction::Restart
            } else {
                BannerAction::Reconnect
            }],
        }
    }

    /// A local shell ended: its code (or signal) and Restart.
    fn shell_exited(status: &ExitStatus) -> Self {
        Self::ended(
            "Shell exited",
            "Shell",
            status,
            BannerAction::Restart,
            RESTART_HINT,
        )
    }

    /// A pod session ended with a verdict: its code, or the server's message when the command
    /// could not run.
    fn session_ended(status: &ExitStatus) -> Self {
        if let (None, None, Some(message)) = (&status.code, &status.signal, &status.message) {
            let failure = Failure::command_failed(message);
            return Self {
                tone: Tone::Error,
                headline: failure.headline().to_owned(),
                detail: with_detail(&failure, SESSION_GONE),
                actions: vec![BannerAction::Reconnect, BannerAction::CloseTab],
            };
        }
        Self::ended(
            "Session ended",
            "Session",
            status,
            BannerAction::Reconnect,
            SESSION_GONE,
        )
    }

    /// `subject` ended with `status`, worded `with_code` when it has a code; code 0 offers Close
    /// first, anything else `retry` first.
    fn ended(
        with_code: &str,
        subject: &str,
        status: &ExitStatus,
        retry: BannerAction,
        detail: &str,
    ) -> Self {
        let (headline, clean) = match (&status.code, &status.signal) {
            (Some(code), _) => (format!("{with_code} with code {code}"), *code == 0),
            (None, Some(signal)) => (format!("{subject} ended by signal {signal}"), false),
            (None, None) => (format!("{subject} ended"), false),
        };
        let (tone, actions) = if clean {
            (Tone::Info, vec![BannerAction::CloseTab, retry])
        } else {
            (Tone::Warning, vec![retry, BannerAction::CloseTab])
        };
        Self {
            tone,
            headline,
            detail: detail.to_owned(),
            actions,
        }
    }
}

/// The failure's hint, the server's message when there is one, then `tail`.
fn with_detail(failure: &Failure, tail: &str) -> String {
    let mut text = failure.hint().to_owned();
    // The hint of a stream that simply closed says all there is to say.
    if let Some(detail) = failure.detail()
        && failure.kind() != FailureKind::StreamClosed
    {
        text.push(' ');
        text.push('(');
        text.push_str(detail.trim_end_matches('.'));
        text.push_str(").");
    }
    if !tail.is_empty() {
        text.push(' ');
        text.push_str(tail);
    }
    text
}
