//! Starting a cluster's events feed on demand, and saying what state it is in.
//!
//! The tool, the mention and "Send to agent" never open a watch of their own: they subscribe to
//! the [`EventService`](crate::events::EventService) like the events view does, so the cluster
//! keeps its one feed (started if nobody else wanted it, stopped after the service's idle grace
//! once the last reader leaves).

use futures::StreamExt as _;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::redact::redact;

use super::bounds::WARM_DEADLINE;
use super::install::EventsAgentDeps;
use crate::events::{EventFilter, EventsState, EventsSubscription};
use crate::tools::metrics::within;

/// A filter nothing matches (no event has this involved-object uid): the subscription then keeps
/// the feed alive and reports its state without collecting a single change.
const KEEP_ALIVE_UID: &str = "\u{0}oxikube:no-such-object";

/// A subscription that holds the cluster's feed open, and what it knew when the call asked.
pub(super) struct OpenFeed {
    _subscription: EventsSubscription,
    /// The feed's state after waiting for a feed that was just started.
    pub(super) state: EventsState,
    /// Whether the wait for the first listing ran out.
    pub(super) timed_out: bool,
}

impl OpenFeed {
    /// Whether no events can be read: the cluster is not connected (nothing is watched), or the
    /// feed failed for good.
    pub(super) fn unavailable(&self) -> bool {
        matches!(self.state, EventsState::Idle | EventsState::Failed { .. })
    }

    /// A sentence for a feed whose events are missing or may be incomplete; `None` for a healthy
    /// one.
    pub(super) fn note(&self) -> Option<String> {
        match &self.state {
            EventsState::Idle => {
                Some("events are not being watched: the cluster is not connected".to_owned())
            }
            EventsState::Failed { message, .. } => {
                Some(format!("events are unavailable: {}", redact(message)))
            }
            EventsState::Retrying { message } => Some(format!(
                "the events feed is reconnecting ({}); the events held may be stale",
                redact(message)
            )),
            EventsState::Warming if self.timed_out => Some(
                "the events feed is still loading; the events listed may be incomplete".to_owned(),
            ),
            EventsState::Warming | EventsState::Ready => None,
        }
    }

    /// The state as one word.
    pub(super) fn word(&self) -> &'static str {
        match self.state {
            EventsState::Idle => "idle",
            EventsState::Warming => "warming",
            EventsState::Ready => "ready",
            EventsState::Retrying { .. } => "retrying",
            EventsState::Failed { .. } => "failed",
        }
    }
}

/// Subscribes to `cluster`'s events (starting the feed when nobody else did) and waits, up to a
/// deadline on the app clock, for a feed that is still warming to list what the cluster holds.
pub(super) async fn open(deps: &EventsAgentDeps, cluster: &ClusterId) -> OpenFeed {
    let mut subscription = deps
        .events
        .subscribe(cluster, EventFilter::new().object(KEEP_ALIVE_UID));
    let mut timed_out = false;
    if subscription.state() == EventsState::Warming {
        let settled = within(deps.clock.as_ref(), WARM_DEADLINE, async {
            while subscription.state() == EventsState::Warming {
                if subscription.next().await.is_none() {
                    break;
                }
            }
        })
        .await;
        timed_out = settled.is_none();
    }
    OpenFeed {
        state: subscription.state(),
        _subscription: subscription,
        timed_out,
    }
}
