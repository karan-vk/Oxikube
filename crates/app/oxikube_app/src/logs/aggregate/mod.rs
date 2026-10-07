//! Multi-pod aggregation (E08-S04): the logs of every pod a workload, a Service or a label
//! selector picks, merged by server timestamp into one session.
//!
//! [`LogService::open_aggregate`](super::LogService::open_aggregate) returns an
//! [`AggregateSession`]: a [`LogSession`](super::LogSession) whose buffer holds the *merged* lines
//! (so the viewer, search and the agent's `get_logs` read it like any session; one ring buffer
//! bounded by `logs.buffer_lines`, not one per pod) plus an [`AggregateView`] with what only a
//! multi-pod viewer needs. Plain async Rust over the [`LogPort`](oxikube_ports::LogPort) and
//! [`ResourceReader`](oxikube_ports::ResourceReader): no gpui, no kube.
//!
//! | Piece | Where |
//! |---|---|
//! | what is read: a workload, a Service or a typed selector, narrowed by label and container | [`AggregateSpec`], [`AggregateSource`] (`spec`) |
//! | `spec.selector` to a label selector string | [`selector_of`], [`and_selectors`] (`selector`) |
//! | which containers of a pod can be read now | `pods` |
//! | the pods and their streams: the cap, the events, one stream per container | `fleet` |
//! | the reorder window: the merge order | `merge` |
//! | the task that drives it all, and the task of one stream | `coordinator`, `stream` |
//! | the streams, pod events and hidden sources, as the viewer reads them | [`AggregateView`], [`AggregateChanges`], [`SourceInfo`], [`PodEvent`], [`HiddenSources`] (`view`, `sources`) |
//!
//! # Data flow
//!
//! The coordinator task reads the object (a Deployment's `spec.selector`, a Service's selector),
//! watches the pods that selector matches and opens one stream task per streamable container (at
//! most `logs.max_streams` at a time; the pods left out are counted for the viewer's "N more pods
//! not streamed" notice). Each stream task reads its container's log in batches (the same
//! batching as a single session) and sends them to the coordinator, which puts them in the
//! [`Merger`](merge) and commits what is due to the session's buffer.
//!
//! # Order
//!
//! Lines are merged by their **server timestamp** (the kubelet's, requested with
//! `timestamps=true`), then by stream id, then by the line's position in its own stream: the same
//! input always merges to the same output, and lines of one pod never change order, whatever the
//! clock skew between nodes. Streams arrive with different latencies, so lines wait in a reorder
//! window ([`LogConfig::reorder_window`](super::LogConfig), 300 ms) before they are committed,
//! and nothing is committed until every stream of the first group answered (or
//! [`startup_wait`](super::LogConfig) passed) and one more window went by: a pod that is slow to
//! open does not find the other pods' newer lines placed before its older ones. A line that arrives later than the window
//! (the backlog of a pod that joined after the view opened) is committed at once, after the lines
//! already there: best effort, never blocking the others.
//!
//! # Pod events
//!
//! When the selector's pod set changes the [`AggregateView`] records a [`PodEvent`] (`Added` for a
//! pod that appears after the view opened, `Ended` for a pod that is deleted or whose streams all
//! ended) and its [`changes`](AggregateView::changes) stream fires: the viewer's banner, and the
//! hook following replacements (E08-S07) builds on. The pods of the first list are the baseline,
//! not "added".

mod coordinator;
mod fleet;
mod merge;
mod pods;
mod selector;
mod session;
mod sources;
mod spec;
mod stream;
#[cfg(test)]
mod tests;
mod view;

pub(super) use coordinator::Coordinator;
pub use selector::{and_selectors, selector_of};
pub use session::{AggregatePorts, AggregateSession};
pub use sources::{HiddenSources, PodChange, PodEvent, SourceId, SourceInfo, SourceState};
pub use spec::{AggregateSource, AggregateSpec, is_aggregate_kind};
pub(super) use view::AggShared;
pub use view::{AggregateChanges, AggregateView};
