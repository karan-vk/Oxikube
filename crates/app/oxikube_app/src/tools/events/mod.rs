//! The events tool, the `@events` mention and "Send to agent" for hosted agents (E14-S08).
//!
//! Agents debugging a cluster read the same events a user reads: from the
//! [`EventService`](crate::events::EventService)'s one buffer per cluster, never from a watch of
//! their own. Plain async Rust over the ports (no gpui, no kube): the MCP server lists and calls
//! these through the same registries the agent panel (E27) offers its agents.
//!
//! | Piece | Where |
//! |---|---|
//! | `k8s.get_events`: filtered, capped events as text rows and JSON | [`GetEventsTool`] (`get_events`, `args`) |
//! | `@events/<kind>/<ns>/<name>`: an object's events as a block | [`EventsContextProvider`], [`EventsMention`] (`provider`, `mention`) |
//! | `events::SendToAgent`: selected events queued as agent context | [`register_commands`] (`send`) |
//! | the services behind all three, and registering them | [`EventsAgentDeps`], [`install`] (`install`) |
//!
//! # `k8s.get_events`
//!
//! Read-only (no risk, no confirmation tier), available on a read-only cluster.
//!
//! | Argument | Meaning |
//! |---|---|
//! | `ns` | events stored in this namespace (as `kubectl -n`); default or `all`: every namespace |
//! | `involved` | `{kind, name}` (a kind word like `pod` or `deploy` works) or `{uid}`: only events about that object |
//! | `since` | a duration back from now (`90s`, `15m`, `2h`, `1d`; up to 7 days) or an RFC 3339 timestamp; default: everything held |
//! | `type` | `Warning` or `Normal`; default both |
//! | `limit` | 1 to [`MAX_LIMIT`], default [`DEFAULT_LIMIT`] |
//!
//! The text is a tab-separated table (`TYPE REASON OBJECT COUNT AGE MESSAGE`, newest first by
//! last seen). The structured result is stable: `status` (`ok` or `unavailable`), `cluster`,
//! `feed`, `total` (every match held), `returned`, `truncated`, `note` and `events`, each
//! `{type, reason, object, namespace, count, first_seen, last_seen, age, message, reporter}`.
//! `truncated: true` with `total` tells the agent to narrow the query.
//!
//! # Absence is an answer
//!
//! An empty buffer is an empty list with a `note` (events live about an hour in the cluster, and
//! only what the app watched is held). A cluster that is not connected, or whose events feed
//! failed for good (no permission, no events API), is `status: "unavailable"` with the reason,
//! never an `Err`. A feed nobody started yet is started by the call (the same ref-counted
//! subscription the events view uses) and the call waits, up to a deadline on the app clock, for
//! its first listing. A malformed call (a bad `since`, an over-limit `limit`, `involved` with
//! half a name) is an `Err`.
//!
//! # Size and redaction
//!
//! Rows are capped ([`MAX_LIMIT`]), messages cut to [`MAX_MESSAGE_CHARS`] characters on one line,
//! and every string that came from the cluster passes through `oxikube_domain::redact` (event
//! messages quote image names with credentials in the URL).

mod args;
mod block;
mod bounds;
mod feed;
mod get_events;
mod install;
mod mention;
mod provider;
mod query;
mod render;
mod send;

#[cfg(test)]
mod tests;

pub use bounds::{
    DEFAULT_LIMIT, MAX_BLOCK_MESSAGE_CHARS, MAX_LIMIT, MAX_MENTION_EVENTS, MAX_MESSAGE_CHARS,
    MAX_SENT_EVENTS,
};
pub use get_events::{GET_EVENTS, GetEventsTool};
pub use install::{EventsAgentDeps, InstallError, install};
pub use mention::EventsMention;
pub use provider::{EVENTS_PREFIX, EventsContextProvider};
pub use send::register_commands;
