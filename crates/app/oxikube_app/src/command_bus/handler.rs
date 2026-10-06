//! [`CommandHandler`]: the code a crate registers for a command id.

use std::future::Future;
use std::sync::Arc;

use futures::future::BoxFuture;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};

use super::context::CommandOutput;
use crate::guard::Mutation;

/// The future a handler returns. Owned and `'static`, so the caller can spawn it.
pub type HandlerFuture = BoxFuture<'static, OxiResult<CommandOutput>>;

/// Runs one command.
///
/// Handlers capture the services they need when they are registered (a session
/// manager, a store) and receive the command payload and a [`HandlerContext`] per call.
/// Any `Fn(Command, HandlerContext) -> impl Future<Output = OxiResult<CommandOutput>>`
/// closure is a handler.
///
/// A handler never blocks and never waits for the UI: it runs on whatever task called
/// [`CommandBus::dispatch`](super::CommandBus::dispatch) (Kubernetes work goes through
/// `oxikube_runtime::spawn_kube` in the caller).
pub trait CommandHandler: Send + Sync {
    /// Runs `command`.
    fn handle(&self, command: Command, cx: HandlerContext) -> HandlerFuture;
}

impl<F, Fut> CommandHandler for F
where
    F: Fn(Command, HandlerContext) -> Fut + Send + Sync,
    Fut: Future<Output = OxiResult<CommandOutput>> + Send + 'static,
{
    fn handle(&self, command: Command, cx: HandlerContext) -> HandlerFuture {
        Box::pin(self(command, cx))
    }
}

/// Per-call context handed to a [`CommandHandler`].
#[derive(Debug)]
pub struct HandlerContext {
    initiator: Initiator,
    who: Arc<str>,
    cluster: Option<ClusterId>,
    mutation: Option<Mutation>,
    dry_run: bool,
}

impl HandlerContext {
    pub(crate) fn new(
        initiator: Initiator,
        who: Arc<str>,
        cluster: Option<ClusterId>,
        mutation: Option<Mutation>,
    ) -> Self {
        Self {
            initiator,
            who,
            cluster,
            mutation,
            dry_run: false,
        }
    }

    /// Marks the call as a dry run (see [`HandlerContext::dry_run`]).
    pub(crate) fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Whether the caller asked for a dry run: the handler must describe what it would do and
    /// change nothing. Mutating handlers read this from their [`Mutation`] instead; it is set
    /// for commands that run outside the mutating pipeline (the posture commands).
    pub fn dry_run(&self) -> bool {
        self.dry_run
    }

    /// Which door the request came through.
    pub fn initiator(&self) -> Initiator {
        self.initiator
    }

    /// The acting identity.
    pub fn who(&self) -> &str {
        &self.who
    }

    /// The cluster the command acts on: the one it names, else the active one.
    pub fn cluster(&self) -> Option<&ClusterId> {
        self.cluster.as_ref()
    }

    /// The guard's write permission. `Some` exactly for mutating commands that passed
    /// the guard; read commands never get one.
    pub fn mutation(&self) -> Option<&Mutation> {
        self.mutation.as_ref()
    }

    /// The guard's write permission, or an `Internal` error for a handler of a mutating
    /// command that was somehow called without one.
    pub fn require_mutation(&self) -> OxiResult<&Mutation> {
        self.mutation
            .as_ref()
            .ok_or_else(|| OxiError::internal("mutating handler called without a guard permit"))
    }
}
