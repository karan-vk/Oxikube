//! The [`CommandBus`]: every user and agent action is a `Command` dispatched here
//! (non-negotiable 4; E06-S02).
//!
//! The palette, the keymap, context menus, buttons, hosted agents (MCP) and extensions
//! all build a [`Command`](oxikube_domain::command::Command) and call
//! [`CommandBus::dispatch`] with a [`DispatchContext`] saying who asked
//! ([`Initiator`](oxikube_domain::audit::Initiator)). The bus finds the handler by
//! [`CommandId`](oxikube_domain::command::CommandId) and, for a mutating command, runs it
//! through the [`MutationGuard`](crate::guard::MutationGuard).
//!
//! # Registration
//!
//! Handlers are registered per crate: each crate's registration function adds its
//! commands to a [`CommandRegistry`] ([`CommandRegistry::install`] records which crate),
//! and `bins/oxikube` builds the bus from it once at startup, so feature crates never
//! edit this one. A registration is rejected when the id is already taken, is not
//! declared in `oxikube_domain::command`, or comes with metadata that differs from its
//! declaration. Every registration also produces the command's MCP tool stub
//! ([`CommandBus::tools`]); privileged commands (lifting read-only mode) get none.
//!
//! # Threading
//!
//! Plain async Rust: the bus spawns nothing and never waits for the UI. Its checks are
//! synchronous; the caller drives the returned future on a background task.

mod bus;
mod context;
mod error;
mod handler;
mod registry;

#[cfg(test)]
mod tests;

pub use bus::CommandBus;
pub use context::{CommandOutput, DispatchContext, Outcome};
pub use error::DispatchError;
pub use handler::{CommandHandler, HandlerContext, HandlerFuture};
pub use registry::{CommandRegistry, RegisterError};
