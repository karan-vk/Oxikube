//! [`ConfirmationProvider`]: the confirmation round trip as one awaitable call, for a caller that
//! can ask its user inline (an agent's permission prompt, E26/E27; a test).
//!
//! The bus itself never waits for anyone: a command that needs confirming returns
//! [`Outcome::NeedsConfirmation`] and the caller dispatches again with the answer. That stays the
//! only protocol. [`CommandBus::execute`] is a thin adapter over it: it dispatches, hands a
//! [`ConfirmationRequest`] to the provider, and dispatches again with its [`Decision`] (or
//! declines). The guard's checks, the token binding (command, payload, initiator, dry-run flag)
//! and the audit records are exactly those of two dispatches by hand.
//!
//! [`ConfirmationRouter`] picks the provider by the request's [`Initiator`]: an agent's request
//! goes to the agent's permission prompt, never to a provider meant for the UI, and an initiator
//! with no provider is declined (fail closed). No lock is held while a provider waits; if the
//! task waiting is dropped, the request is forgotten and audited as cancelled.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;

use super::MutationGuard;
use super::confirm::{Confirmation, ConfirmationRequest, ConfirmationToken};
use crate::command_bus::{CommandBus, DispatchContext, DispatchError, Outcome};

/// A provider's answer to a [`ConfirmationRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run it. For a [`ConfirmTier::TypeName`](oxikube_domain::safety::ConfirmTier::TypeName)
    /// request, `typed_name` is what the user typed; the guard compares it.
    Confirm {
        /// What the user typed, if the request asked for a name.
        typed_name: Option<String>,
    },
    /// Do not run it.
    Decline,
}

impl Decision {
    /// A one-click confirmation.
    pub fn confirm() -> Self {
        Decision::Confirm { typed_name: None }
    }

    /// A type-the-name confirmation.
    pub fn typed(name: impl Into<String>) -> Self {
        Decision::Confirm {
            typed_name: Some(name.into()),
        }
    }
}

/// Asks a person (or the policy standing in for one) whether a guarded mutation may run.
///
/// Implemented by an agent's permission layer (E27) and by tests; the UI answers through its
/// dialog and a second dispatch instead. A provider must not hold a lock the guard needs while it
/// waits, and should honour [`ConfirmationRequest::expected_name`] by asking for the name.
#[async_trait]
pub trait ConfirmationProvider: Send + Sync {
    /// The answer to `request`.
    async fn confirm(&self, request: &ConfirmationRequest) -> Decision;
}

/// Routes each request to the provider registered for its [`Initiator`]; an initiator with none
/// is declined.
#[derive(Clone, Default)]
pub struct ConfirmationRouter {
    providers: HashMap<Initiator, Arc<dyn ConfirmationProvider>>,
}

impl std::fmt::Debug for ConfirmationRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfirmationRouter")
            .field("initiators", &self.providers.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ConfirmationRouter {
    /// A router with no providers: it declines everything.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends the requests of `initiator` to `provider`.
    #[must_use]
    pub fn with(mut self, initiator: Initiator, provider: Arc<dyn ConfirmationProvider>) -> Self {
        self.providers.insert(initiator, provider);
        self
    }
}

#[async_trait]
impl ConfirmationProvider for ConfirmationRouter {
    async fn confirm(&self, request: &ConfirmationRequest) -> Decision {
        match self.providers.get(&request.initiator) {
            Some(provider) => provider.confirm(request).await,
            None => {
                tracing::warn!(
                    command = %request.command,
                    initiator = %request.initiator,
                    "no confirmation provider for this initiator; declined"
                );
                Decision::Decline
            }
        }
    }
}

/// Abandons its token on drop unless disarmed: the dispatch waiting on a provider was dropped.
struct Unanswered<'a> {
    guard: &'a MutationGuard,
    token: ConfirmationToken,
    armed: bool,
}

impl Unanswered<'_> {
    /// The provider answered: the token is the caller's again.
    fn disarm(mut self) -> ConfirmationToken {
        self.armed = false;
        self.token
    }
}

impl Drop for Unanswered<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.guard.abandon(self.token);
        }
    }
}

impl CommandBus {
    /// Runs `command` end to end, asking `provider` when the guard needs a confirmation: the
    /// whole pipeline (read-only check, confirmation tier, the mandatory dry run, the handler,
    /// the audit record) in one call: dispatch, ask `provider`, dispatch again with its answer or
    /// decline. The guard's checks and records are exactly those of the two dispatches by hand;
    /// if this future is dropped while `provider` is deciding, the request is forgotten and
    /// audited as cancelled by the user.
    ///
    /// # Errors
    ///
    /// Those of [`dispatch`](Self::dispatch), and [`DispatchError::Declined`] when the provider
    /// declined (nothing ran; the attempt is audited as cancelled by the user).
    pub async fn execute(
        &self,
        command: Command,
        ctx: DispatchContext,
        provider: &dyn ConfirmationProvider,
    ) -> Result<Outcome, DispatchError> {
        let id = command.id();
        let request = match self.dispatch(command.clone(), ctx.clone()).await? {
            Outcome::NeedsConfirmation(request) => request,
            done @ Outcome::Completed(_) => return Ok(done),
        };
        let unanswered = Unanswered {
            guard: self.guard(),
            token: request.token,
            armed: true,
        };
        let decision = provider.confirm(&request).await;
        let token = unanswered.disarm();
        match decision {
            Decision::Confirm { typed_name } => {
                let answer = Confirmation { token, typed_name };
                self.dispatch(command, ctx.with_confirmation(answer)).await
            }
            Decision::Decline => {
                self.decline(token).await?;
                Err(DispatchError::Declined(id))
            }
        }
    }
}
