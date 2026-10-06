//! What goes into a dispatch ([`DispatchContext`]) and what comes out ([`Outcome`]).

use std::sync::Arc;

use oxikube_domain::audit::Initiator;
use oxikube_domain::ids::ClusterId;
use serde_json::Value;

use crate::guard::{Confirmation, ConfirmationRequest};

/// Who is dispatching, on which cluster, and with which answers.
///
/// The palette and keymap build it with [`Initiator::Command`], buttons and menus with
/// [`Initiator::Ui`], the MCP server with [`Initiator::Agent`] and the extension host
/// with [`Initiator::Plugin`]. Every initiator gets the same read-only block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchContext {
    /// Which door the request came through; recorded in the audit record.
    pub initiator: Initiator,
    /// The acting identity: the local user, or the agent or plugin name. Redacted
    /// before it is audited.
    pub who: Arc<str>,
    /// The active cluster, used when the command does not name one itself.
    pub cluster: Option<ClusterId>,
    /// The answer to an earlier [`Outcome::NeedsConfirmation`].
    pub confirmation: Option<Confirmation>,
    /// Ask the handler for a server-side dry run (recorded in the audit record).
    pub dry_run: bool,
}

impl DispatchContext {
    /// A context for `initiator` acting as `who`, with nothing else set.
    pub fn new(initiator: Initiator, who: impl Into<Arc<str>>) -> Self {
        Self {
            initiator,
            who: who.into(),
            cluster: None,
            confirmation: None,
            dry_run: false,
        }
    }

    /// Sets the active cluster.
    #[must_use]
    pub fn with_cluster(mut self, cluster: ClusterId) -> Self {
        self.cluster = Some(cluster);
        self
    }

    /// Answers a confirmation request.
    #[must_use]
    pub fn with_confirmation(mut self, confirmation: Confirmation) -> Self {
        self.confirmation = Some(confirmation);
        self
    }

    /// Asks for a server-side dry run.
    #[must_use]
    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }
}

/// What a successful dispatch returned.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The handler ran.
    Completed(CommandOutput),
    /// The command is a mutation that needs the user's confirmation first. Nothing ran
    /// and nothing was audited yet; dispatch the same command again with
    /// [`DispatchContext::with_confirmation`], or decline it.
    NeedsConfirmation(ConfirmationRequest),
}

impl Outcome {
    /// The output, if the handler ran.
    pub fn completed(self) -> Option<CommandOutput> {
        match self {
            Outcome::Completed(output) => Some(output),
            Outcome::NeedsConfirmation(_) => None,
        }
    }

    /// The confirmation request, if one is needed.
    pub fn confirmation(self) -> Option<ConfirmationRequest> {
        match self {
            Outcome::NeedsConfirmation(request) => Some(request),
            Outcome::Completed(_) => None,
        }
    }
}

/// What a handler returns: an optional message for a toast or the agent, and optional
/// structured data (the tool's `structuredContent`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CommandOutput {
    /// A short human message.
    pub message: Option<String>,
    /// Structured result.
    pub data: Option<Value>,
}

impl CommandOutput {
    /// No output.
    pub fn none() -> Self {
        Self::default()
    }

    /// A message.
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: Some(message.into()),
            data: None,
        }
    }

    /// Structured data.
    pub fn data(data: Value) -> Self {
        Self {
            message: None,
            data: Some(data),
        }
    }
}
