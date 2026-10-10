//! Immediate commands: handlers whose visible effect runs on the caller's thread, in the
//! dispatching call (E05-P600).
//!
//! A command that only changes UI or session state in memory (the cluster tab order, the
//! namespace selection) gains nothing from a trip through a background task, and loses a frame
//! to it: the UI dispatches during an update and the async answer comes back in a later one, so
//! the frame drawn right after the input cannot show it. Such a command registers an
//! [`ImmediateHandler`] ([`CommandRegistry::register_immediate`](super::CommandRegistry::register_immediate)).
//! The UI runs it with [`CommandBus::dispatch_now`](super::CommandBus::dispatch_now) inside its
//! own update; any I/O that completes the command (remembering the selection) comes back as
//! [`Immediate::rest`], for the caller to run off the UI thread.
//!
//! Only commands the guard has no pipeline for can be immediate: never a mutation, an exec, a
//! privileged or a posture command (their guard pipelines audit asynchronously). Every other
//! caller ([`CommandBus::dispatch`](super::CommandBus::dispatch): MCP, plugins) runs the same
//! handler and awaits its rest before answering, so an immediate command behaves the same from
//! every door.

use std::sync::Arc;

use futures::future::BoxFuture;
use oxikube_domain::OxiResult;
use oxikube_domain::command::Command;

use super::context::CommandOutput;
use super::handler::{CommandHandler, HandlerContext, HandlerFuture};

/// What an immediate handler did: its output, and the I/O that completes the command, if any.
#[must_use = "the command is not complete until `rest` has run"]
pub struct Immediate {
    /// The handler's output.
    pub output: CommandOutput,
    /// The I/O that completes the command (a state store write). The UI runs it off the UI
    /// thread (`oxikube_runtime::spawn_kube`); `CommandBus::dispatch` awaits it.
    pub rest: Option<BoxFuture<'static, OxiResult<()>>>,
}

impl std::fmt::Debug for Immediate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Immediate")
            .field("output", &self.output)
            .field("rest", &self.rest.is_some())
            .finish()
    }
}

impl Immediate {
    /// A command that is complete with `output`.
    pub fn done(output: CommandOutput) -> Self {
        Self { output, rest: None }
    }

    /// A command whose visible part is done, with `rest` still to run.
    pub fn then(
        output: CommandOutput,
        rest: impl Future<Output = OxiResult<()>> + Send + 'static,
    ) -> Self {
        Self {
            output,
            rest: Some(Box::pin(rest)),
        }
    }
}

/// Runs one command on the caller's thread: in memory, no I/O, no waiting (see the
/// [module docs](self)). Any
/// `Fn(Command, HandlerContext) -> OxiResult<Immediate>` closure is one.
pub trait ImmediateHandler: Send + Sync {
    /// Runs `command` now.
    fn handle_now(&self, command: Command, cx: HandlerContext) -> OxiResult<Immediate>;
}

impl<F> ImmediateHandler for F
where
    F: Fn(Command, HandlerContext) -> OxiResult<Immediate> + Send + Sync,
{
    fn handle_now(&self, command: Command, cx: HandlerContext) -> OxiResult<Immediate> {
        self(command, cx)
    }
}

/// An immediate handler seen as an async one: runs it, then awaits its rest. What
/// `CommandBus::dispatch` calls.
pub(super) struct AwaitRest(pub(super) Arc<dyn ImmediateHandler>);

impl CommandHandler for AwaitRest {
    fn handle(&self, command: Command, cx: HandlerContext) -> HandlerFuture {
        let handler = self.0.clone();
        Box::pin(async move {
            let Immediate { output, rest } = handler.handle_now(command, cx)?;
            if let Some(rest) = rest {
                rest.await?;
            }
            Ok(output)
        })
    }
}
