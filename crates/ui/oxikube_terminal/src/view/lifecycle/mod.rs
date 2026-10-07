//! What state a terminal is in, as a small enum a test can drive without a window (E09-S12).
//!
//! ```text
//!            Started                 Transport error (pod)        Relaunch
//! Connecting ───────▶ Running ─────────────────────────▶ Disconnected ─────▶ Connecting
//!     │                  │  Exited                          │ Exited (code)
//!     │ StartFailed      ├──────────────────▶ Exited ───────┘     │ Relaunch
//!     ▼                  ▼                       └────────────────┴────────▶ Connecting
//!   Failed ──Relaunch──▶ Connecting                       any ──Close──▶ Closed
//! ```
//!
//! | Module | What |
//! |---|---|
//! | `failure` | [`Failure`]: the error taxonomy, one headline and hint per kind |
//! | `banner` | [`Banner`]: the text and actions the lifecycle asks the view to show |
//!
//! The [`TerminalView`](super::TerminalView) feeds [`Lifecycle::apply`] with what it sees (the
//! launch result, the session's [`TerminalEvent`](crate::TerminalEvent)s) and draws
//! [`Lifecycle::banner`]. A pod terminal whose transport fails is *disconnected* (the grid stays,
//! input stops, Reconnect opens a new session); a local shell's end is an *exit* (the grid stays,
//! Restart starts a fresh shell). Both leave the old session's screen readable.

mod banner;
mod failure;

use oxikube_ports::ExitStatus;

pub use banner::{Banner, BannerAction, Tone};
pub use failure::{Failure, FailureKind};

/// Where a terminal is in its life. See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lifecycle {
    /// The launcher is starting (or restarting) the process.
    Connecting,
    /// The process runs and takes input.
    Running,
    /// A pod session lost its connection; the screen is the last it showed.
    Disconnected(Failure),
    /// The process ended with `status`; the screen stays readable.
    Exited(ExitStatus),
    /// The process could not start.
    Failed(Failure),
    /// The tab closed.
    Closed,
}

/// What the view tells the [`Lifecycle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// The launcher produced a running process.
    Started,
    /// The launcher failed.
    StartFailed(Failure),
    /// The running session reported a transport error.
    Transport(Failure),
    /// The session ended with `status`.
    Exited(ExitStatus),
    /// The user asked for a new session (Reconnect / Restart).
    Relaunch,
    /// The tab closed.
    Close,
}

impl Lifecycle {
    /// The state after `signal`. `local` is whether the process runs on this machine: a local
    /// shell has no connection to lose, so a transport error there changes nothing (its end is an
    /// exit). A signal that makes no sense in a state (an exit before the start, anything after
    /// the close) leaves it as it is.
    #[must_use]
    pub fn apply(self, signal: Signal, local: bool) -> Self {
        match (self, signal) {
            (Self::Closed, _) | (_, Signal::Close) => Self::Closed,
            (Self::Connecting, Signal::Started) => Self::Running,
            (Self::Connecting, Signal::StartFailed(failure)) => Self::Failed(failure),
            (Self::Running, Signal::Transport(failure)) if !local => Self::Disconnected(failure),
            (Self::Running, Signal::Exited(status)) => Self::ended(None, status, local),
            (Self::Disconnected(failure), Signal::Exited(status)) => {
                Self::ended(Some(failure), status, local)
            }
            (state, Signal::Relaunch) if state.can_relaunch() => Self::Connecting,
            (state, _) => state,
        }
    }

    /// How a session that was running (or had lost its connection, `lost`) ended with `status`.
    ///
    /// A pod session whose stream ends with neither an exit code nor a signal had no verdict: it
    /// was cut. After a transport error it stays *disconnected* (the exit that follows an error is
    /// only the stream ending); without one the connection was closed from the other side. A
    /// status with a code or a signal is always the verdict, a command that could not run (a
    /// message and no code) is an exit that says so.
    fn ended(lost: Option<Failure>, status: ExitStatus, local: bool) -> Self {
        let verdict = status.code.is_some() || status.signal.is_some();
        if local || verdict || (lost.is_none() && status.message.is_some()) {
            return Self::Exited(status);
        }
        Self::Disconnected(lost.unwrap_or_else(|| Failure::new(FailureKind::StreamClosed)))
    }

    /// Whether the process takes input: only while it runs. A disconnected or ended session's
    /// keystrokes go nowhere, and the view dims its screen to say so.
    pub fn accepts_input(&self) -> bool {
        matches!(self, Self::Running)
    }

    /// Whether Reconnect / Restart can start a new session: after a drop, an exit or a failed
    /// start.
    pub fn can_relaunch(&self) -> bool {
        matches!(
            self,
            Self::Disconnected(_) | Self::Exited(_) | Self::Failed(_)
        )
    }

    /// The banner to show above the screen, `None` while it runs (or starts, or is closed).
    /// `local` is whether the process runs on this machine (a shell restarts, a pod reconnects).
    pub fn banner(&self, local: bool) -> Option<Banner> {
        Banner::of(self, local)
    }
}

#[cfg(test)]
mod tests;
