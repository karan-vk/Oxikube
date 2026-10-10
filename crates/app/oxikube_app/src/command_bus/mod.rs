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
//! # Introspection
//!
//! The palette, the help overlay and the keymap do not dispatch; they ask what exists. The bus
//! keeps a [`CommandIndex`] of its commands, sorted by category then title, and answers
//! [`CommandBus::list`] (the commands that can run in a [`CommandContext`]: the focused view,
//! the session's read-only flag and capabilities, the selection), [`CommandBus::all`] (every
//! registered command, for a "show all" toggle) and [`CommandBus::get`]. Each command's
//! `availability` is plain data in `oxikube_domain::command`, evaluated by [`check`]; a
//! mutation is *hidden* from `list` on a read-only session (the guard would refuse it), and
//! [`CommandInfo::check`] says why it is unavailable for a "show all" row.
//!
//! # Threading
//!
//! Plain async Rust: the bus spawns nothing and never waits for the UI. Its checks are
//! synchronous; the caller drives the returned future on a background task.
//!
//! # Immediate commands
//!
//! A command that only changes UI or session state in memory (the cluster tabs, the namespace
//! selection) registers an [`ImmediateHandler`] instead
//! ([`CommandRegistry::register_immediate`]). The UI runs it inside the update that dispatched
//! it with [`CommandBus::dispatch_now`], so its effect is in the very next frame, and runs the
//! I/O that completes it ([`Immediate::rest`]) off the UI thread. Guarded commands can never be
//! immediate. See [`immediate`].

mod availability;
mod bus;
mod context;
mod error;
mod handler;
pub mod immediate;
mod index;
mod registry;

#[cfg(test)]
mod tests;

pub use availability::{CommandContext, Selection, Unavailable, check};
pub use bus::CommandBus;
pub use context::{CommandOutput, DispatchContext, Outcome};
pub use error::DispatchError;
pub use handler::{CommandHandler, HandlerContext, HandlerFuture};
pub use immediate::{Immediate, ImmediateHandler};
pub use index::{CommandIndex, CommandInfo, DuplicateCommand};
pub use registry::{CommandRegistry, RegisterError};
