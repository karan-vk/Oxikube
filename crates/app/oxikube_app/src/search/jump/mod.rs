//! The `:` jump bar's grammar, parser and planner (E11-S05): `:pods`, `:deploy kube-system`,
//! `:pod /re`, `:pod app=x @prod`, `:ctx prod`, `:ns`, `:q`.
//!
//! | Module | Holds |
//! |---|---|
//! | `parse` | [`parse`]: a hand-written tokenizer and a loop, line to [`JumpCommand`] or [`ParseError`] with a [`Span`] to underline; cheap enough for every keystroke |
//! | `ast`, `span`, `error` | [`JumpCommand`], [`ResourceJump`], [`RawFilter`] (the `/filter`, uninterpreted: the filter bar's grammar parses it), [`HistoryStep`]; `Display` is the canonical form, which parses back to an equal tree |
//! | `plan`, `lookup`, `env` | [`plan`] / [`resolve`]: a command resolved against a [`JumpEnv`] (the cluster's alias table, namespaces and contexts) into the navigation [`Command`](oxikube_domain::command::Command)s that do it ([`JumpPlan`]) |
//! | `history` | [`JumpHistory`]: the session's ring, with `-` (previous view), `[` (back) and `]` (forward) |
//! | `complete` | [`site`] and [`candidates`]: the word being typed and what it could be, for the bar's list |
//!
//! # Grammar
//!
//! ```text
//! :<alias> [ns] [/filter] [k=v,..] [@ctx]     a resource list
//! :ctx [name]     :ns [name]     :q           context, namespaces, quit
//! :-   :[   :]                                 history
//! ```
//!
//! The first word is an alias (the table of `oxikube_app::search::aliases`) or a reserved word.
//! After it, a word starting with `/` is the filter (`/-l`, `/-f` and `/!-f` take the next word as
//! their operand), one starting with `@` the context, one containing `=` a label selector, and any
//! other word the namespace (`all` for every namespace). Each at most once, in any order.
//!
//! # Errors
//!
//! Every error carries the [`Span`] of the offending word: an unknown alias (with close names), a
//! second filter, a selector with an empty term, a trailing `@`, an unknown namespace (against the
//! cluster's own list, when it has been read) or context.
//!
//! # Execution
//!
//! A [`JumpPlan`] is data: the bar sends each of its commands through the `CommandBus`, so a jump
//! is guarded, audited and available to an agent like any click (non-negotiable 4). Nothing in
//! this module touches a window; the bar (`oxikube_palette::jump`) owns the rest.

mod ast;
mod complete;
mod env;
mod error;
mod history;
mod lookup;
mod parse;
mod plan;
mod span;
mod token;

#[cfg(test)]
mod tests;

pub use ast::{HistoryStep, JumpCommand, RawFilter, ResourceJump};
pub use complete::{Candidate, CompletionSite, Slot, accept, candidates, site};
pub use env::{JumpContext, JumpEnv};
pub use error::{ParseError, ParseErrorKind};
pub use history::{CAPACITY as HISTORY_CAPACITY, JumpHistory};
pub use parse::parse;
pub use plan::{AfterConnect, CATALOG_VIEW_ID, JumpPlan, plan, resolve};
pub use span::{Span, Spanned};
