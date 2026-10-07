//! Reconnect and churn following (E08-S07): a log keeps flowing when its connection drops, says
//! why it ended when its pod goes, and finds the pod that replaced it.
//!
//! | Piece | Where |
//! |---|---|
//! | backoff with jitter, the retry cap (`logs.reconnect_retries`) | [`ReconnectPolicy`], [`Backoff`] (`policy`) |
//! | dropping the lines a reopened stream replays: (server timestamp, text hash) over the last 512 lines | `overlap` |
//! | the read loop: reopen from the last line minus the overlap, retry a container that is still starting, give up after the cap | `resume` |
//! | why a followed pod's stream ended: finished, replaced, deleted | [`PodIdentity`], `probe` |
//! | the followed container in a pod that runs on: finished for good, or between restarts | `container` |
//! | the pod that took over from a gone one (owner, selector, same name / node, newest) | [`find_replacement`] (`replacement`) |
//!
//! # Single-pod sessions
//!
//! A session opened with [`LogService::open_following_in`](super::LogService::open_following_in)
//! reads its pod's [`PodIdentity`] next to the stream. When the stream ends it reads the pod again:
//! still running is a dropped connection (`Reconnecting n/m`, then `Streaming` again with the
//! overlap removed); `Succeeded`/`Failed` is [`EndReason::PodFinished`](super::EndReason); a
//! followed container that exited for good in a pod that runs on (a completed init container) is
//! [`EndReason::ContainerFinished`](super::EndReason), and one between restarts
//! (`CrashLoopBackOff`) is waited for (`Connecting`, pauses that grow, no retry counted); deleted,
//! terminating or recreated under its name is [`EndReason::PodReplaced`](super::EndReason) when a
//! controller owns it (the viewer offers "follow replacement", which [`find_replacement`] answers)
//! and [`EndReason::PodDeleted`](super::EndReason) otherwise. A denied read is `Failed` at once;
//! retries that run out are `Failed` too, and [`LogSession::reconnect`](super::LogSession::reconnect)
//! starts again from the lines the buffer kept.
//!
//! # Multi-pod sessions
//!
//! The aggregate's pod watch already starts a stream for a pod that appears and marks the pod
//! ended when it goes (its lines stay). Here a pod that joins after the view opened is read from
//! its first line (it is new: a tail or a `since` would cut it), and each stream reconnects like a
//! single session's (`SourceState::Reconnecting`), so a rollout restart keeps the view following
//! the new pods with no line twice.

mod container;
mod overlap;
mod policy;
mod probe;
mod replacement;
mod resume;

pub(crate) use overlap::Overlap;
pub use policy::{
    Backoff, DEFAULT_RECONNECT_RETRIES, MAX_RECONNECT_RETRIES, ReconnectPolicy,
    clamp_reconnect_retries,
};
pub use probe::PodIdentity;
pub(crate) use probe::read_pod;
pub use replacement::find_replacement;
pub(crate) use resume::{Finish, IdentityCell, Phase, Probe, Resumable};

#[cfg(test)]
mod tests;
