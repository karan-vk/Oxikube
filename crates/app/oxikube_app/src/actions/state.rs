//! What a session allows: [`ActionContext`], and the [`ActionState`] it gives an action.

use std::fmt;

use oxikube_domain::Capabilities;
use oxikube_domain::command::CommandMeta;

use crate::ClusterSession;

/// The part of a session that decides whether an action can run: what the user may do on the
/// cluster (probed on connect) and whether it is read-only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActionContext {
    /// What the session can do.
    pub capabilities: Capabilities,
    /// Whether the cluster refuses every mutation.
    pub read_only: bool,
    /// Whether a shell, attach or exec still opens on a read-only cluster (the cluster's
    /// `exec_in_read_only` setting).
    pub exec_in_read_only: bool,
}

impl ActionContext {
    /// A context of `capabilities`, not read-only.
    pub fn new(capabilities: Capabilities) -> Self {
        Self {
            capabilities,
            read_only: false,
            exec_in_read_only: false,
        }
    }

    /// The context of `session`.
    pub fn of(session: &ClusterSession) -> Self {
        Self {
            capabilities: session.capabilities(),
            read_only: session.read_only(),
            exec_in_read_only: session.prefs().exec_in_read_only,
        }
    }

    /// The same context with the read-only flag set to `read_only`.
    #[must_use]
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// The same context with `exec_in_read_only` set.
    #[must_use]
    pub fn exec_in_read_only(mut self, allowed: bool) -> Self {
        self.exec_in_read_only = allowed;
        self
    }

    /// Whether an action described by `meta` can run here: a mutation and, unless the cluster
    /// allows it, an exec-class command (a shell, an attach) are off on a read-only cluster.
    pub fn state_of(&self, meta: &CommandMeta) -> ActionState {
        if meta.mutating && self.read_only {
            ActionState::Disabled(DisabledReason::ReadOnly)
        } else if meta.exec && self.read_only && !self.exec_in_read_only {
            ActionState::Disabled(DisabledReason::ExecReadOnly)
        } else {
            ActionState::Enabled
        }
    }
}

/// Whether an offered action can run now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionState {
    /// It can.
    Enabled,
    /// It is shown greyed out, with the reason.
    Disabled(DisabledReason),
}

impl ActionState {
    /// Whether the action can run.
    pub fn is_enabled(&self) -> bool {
        matches!(self, ActionState::Enabled)
    }

    /// The reason it cannot run, if it cannot.
    pub fn reason(&self) -> Option<DisabledReason> {
        match self {
            ActionState::Enabled => None,
            ActionState::Disabled(reason) => Some(*reason),
        }
    }
}

/// Why an offered action is disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisabledReason {
    /// The cluster is in read-only mode.
    ReadOnly,
    /// The cluster is read-only and shells into pods are not allowed there
    /// (`exec_in_read_only`).
    ExecReadOnly,
}

impl fmt::Display for DisabledReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DisabledReason::ReadOnly => f.write_str("This cluster is read-only"),
            DisabledReason::ExecReadOnly => f.write_str(
                "This cluster is read-only: shells are blocked (set exec_in_read_only to allow them)",
            ),
        }
    }
}
