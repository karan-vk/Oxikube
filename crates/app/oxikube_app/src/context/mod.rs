//! Agent context (E08-S09): the [`ContextRegistry`] that resolves `@`-mentions to
//! [`ContextBlock`](oxikube_domain::agent::ContextBlock)s, the `@logs` provider, and the queue
//! "Send to agent" fills until the agent panel (E27) exists.
//!
//! Plain async Rust over the [`ContextProviderPort`](oxikube_ports::ContextProviderPort): no gpui,
//! no kube, no `agent-client-protocol` (ADR 0008). The same providers serve the agent panel's
//! mentions and the hosted agents' MCP resources.
//!
//! | Piece | Where |
//! |---|---|
//! | providers by mention prefix; one resolution within a byte budget | [`ContextRegistry`] (`registry`) |
//! | `@logs/<ns>/<pod>[/--since/10m]` and a viewer's selection as a block | [`LogContextProvider`], [`LogMention`], [`selection_context`] (`logs`) |
//! | context waiting for the agent panel, with its source | [`PendingContext`], [`QueuedContext`], [`ContextSource`], [`ContextConsumer`] (`pending`) |
//!
//! # Redaction and budgets
//!
//! A provider masks secrets before it builds a block (non-negotiable 5): `@logs` and the selection
//! pass through `oxikube_domain::redact`, best-effort for free text. A block is at most 64 KiB and
//! says in a `# note:` line when lines were left out and how to get them.

mod logs;
mod pending;
mod registry;

pub use logs::{LOGS_PREFIX, LogContextProvider, LogMention, selection_context};
pub use pending::{
    CollectingConsumer, ConsumerGuard, ContextConsumer, ContextSource, MAX_PENDING_ITEMS,
    PendingContext, QueuedContext, Sent,
};
pub use registry::{ContextRegistry, RegisterProviderError};
