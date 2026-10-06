//! Confirmation round trip: [`ConfirmationRequest`] out, [`Confirmation`] back.
//!
//! The bus never waits for the UI. A mutating command that needs confirming returns
//! [`Outcome::NeedsConfirmation`](crate::command_bus::Outcome::NeedsConfirmation) with a
//! single-use [`ConfirmationToken`]; the UI (or the agent permission prompt) resolves it
//! and dispatches the *same* command again with a [`Confirmation`] in the context, or
//! declines it through [`CommandBus::decline`](crate::command_bus::CommandBus::decline).

use std::sync::atomic::{AtomicU64, Ordering};

use indexmap::IndexMap;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::safety::{ConfirmTier, Risk};
use parking_lot::Mutex;

/// Most confirmations waiting for an answer. Issuing one more forgets the oldest, whose
/// token then fails as [`ConfirmationError::Unknown`].
pub const MAX_PENDING_CONFIRMATIONS: usize = 64;

/// Names one pending confirmation. Only the guard issues tokens; a token is bound to the
/// exact command and initiator it was issued for and is consumed by its first use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConfirmationToken(u64);

/// What the guard needs the user to confirm before a mutation runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationRequest {
    /// Pass back in [`Confirmation::token`].
    pub token: ConfirmationToken,
    /// The command waiting.
    pub command: CommandId,
    /// [`ConfirmTier::Simple`] or [`ConfirmTier::TypeName`].
    pub tier: ConfirmTier,
    /// The command's blast radius.
    pub risk: Option<Risk>,
    /// The cluster it will act on.
    pub cluster: ClusterId,
    /// One line for the dialog: title, target and cluster context.
    pub summary: String,
    /// For [`ConfirmTier::TypeName`]: the text the user must type.
    pub expected_name: Option<String>,
}

/// The user's answer to a [`ConfirmationRequest`], carried in the second dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    /// The token from the request.
    pub token: ConfirmationToken,
    /// What the user typed, for a [`ConfirmTier::TypeName`] request.
    pub typed_name: Option<String>,
}

impl Confirmation {
    /// A one-click confirmation ([`ConfirmTier::Simple`]).
    pub fn simple(token: ConfirmationToken) -> Self {
        Self {
            token,
            typed_name: None,
        }
    }

    /// A type-the-name confirmation ([`ConfirmTier::TypeName`]).
    pub fn typed(token: ConfirmationToken, typed_name: impl Into<String>) -> Self {
        Self {
            token,
            typed_name: Some(typed_name.into()),
        }
    }
}

/// Why a [`Confirmation`] was not accepted. The token is consumed either way: the UI
/// dispatches the command again without one to get a fresh request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfirmationError {
    /// No pending confirmation has this token (never issued, used, declined or evicted).
    #[error("the confirmation is unknown or was already used")]
    Unknown,
    /// The token was issued for a different command, payload, initiator or dry-run flag.
    #[error("the confirmation was issued for a different request")]
    Mismatch,
    /// The typed text is not the name the request asked for.
    #[error("the typed name does not match {expected:?}")]
    NameMismatch {
        /// The name the request asked for.
        expected: String,
    },
}

/// A request waiting for its answer.
#[derive(Debug)]
pub(crate) struct Pending {
    pub(crate) command: Command,
    pub(crate) initiator: Initiator,
    pub(crate) who: std::sync::Arc<str>,
    pub(crate) cluster: ClusterId,
    pub(crate) expected_name: Option<String>,
    pub(crate) dry_run: bool,
}

/// The guard's table of pending confirmations.
#[derive(Debug, Default)]
pub(crate) struct PendingConfirmations {
    next: AtomicU64,
    pending: Mutex<IndexMap<ConfirmationToken, Pending>>,
}

impl PendingConfirmations {
    /// Stores `pending` and returns its new token.
    pub(crate) fn issue(&self, pending: Pending) -> ConfirmationToken {
        let token = ConfirmationToken(self.next.fetch_add(1, Ordering::Relaxed) + 1);
        let mut table = self.pending.lock();
        while table.len() >= MAX_PENDING_CONFIRMATIONS {
            table.shift_remove_index(0);
        }
        table.insert(token, pending);
        token
    }

    /// Removes and returns the pending request for `token`.
    pub(crate) fn take(&self, token: ConfirmationToken) -> Option<Pending> {
        self.pending.lock().shift_remove(&token)
    }

    /// Consumes `answer` for `command` dispatched by `initiator` (as a dry run or not).
    pub(crate) fn redeem(
        &self,
        answer: &Confirmation,
        command: &Command,
        initiator: Initiator,
        dry_run: bool,
    ) -> Result<(), ConfirmationError> {
        let pending = self.take(answer.token).ok_or(ConfirmationError::Unknown)?;
        if pending.command != *command
            || pending.initiator != initiator
            || pending.dry_run != dry_run
        {
            return Err(ConfirmationError::Mismatch);
        }
        match pending.expected_name {
            Some(expected) if answer.typed_name.as_deref() != Some(expected.as_str()) => {
                Err(ConfirmationError::NameMismatch { expected })
            }
            _ => Ok(()),
        }
    }

    /// How many confirmations are waiting.
    pub(crate) fn len(&self) -> usize {
        self.pending.lock().len()
    }
}
