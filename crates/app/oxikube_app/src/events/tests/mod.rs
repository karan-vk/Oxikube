//! Tests of the events service: the buffer and its indexes (with a property test against a
//! naive model), the filters and queries, the coalescing of changes, and the service's feed
//! lifecycle against `FakeEventsPort` and fake sessions.

mod buffer;
mod changes;
mod filter;
mod props;
mod service;
mod timing;

use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::event::Event;
use oxikube_testkit::event;

pub(super) fn t0() -> Timestamp {
    "2026-01-01T00:00:00Z".parse().expect("a timestamp")
}

/// An event `uid` in namespace `ns` about pod `object` (uid `object`), last seen `secs` after
/// [`t0`], `Normal` with reason `Pulled`.
pub(super) fn ev(uid: &str, ns: &str, object: &str, secs: i64) -> Event {
    let mut e = event()
        .uid(uid)
        .reason("Pulled")
        .message(&format!("message of {uid}"))
        .regarding("Pod", Some(ns), object, object)
        .build();
    e.namespace = Some(Arc::from(ns));
    e.last_seen = Some(t0() + SignedDuration::from_secs(secs));
    e
}

/// [`ev`] as a `Warning` with `reason`.
pub(super) fn warning(uid: &str, ns: &str, object: &str, secs: i64, reason: &str) -> Event {
    let mut e = ev(uid, ns, object, secs);
    e.event_type = oxikube_domain::event::EventType::Warning;
    e.reason = Arc::from(reason);
    e
}

/// The uids of `events`, in order.
pub(super) fn uids<'a>(events: impl IntoIterator<Item = &'a Arc<Event>>) -> Vec<String> {
    events
        .into_iter()
        .map(|e| e.uid.as_deref().unwrap_or("-").to_owned())
        .collect()
}
