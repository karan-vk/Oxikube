//! The adaptive interval: 15 s while polls answer, backoff on errors and 503s (reset on the next
//! answer), a slow cadence while the API is absent, at most one interval after a resume.

use std::time::Duration;

use oxikube_domain::OxiError;
use oxikube_domain::metrics::MissingReason;
use oxikube_ports::MetricsOutcome;

use super::{at, t0};
use crate::metrics::live::{Cadence, DEFAULT_POLL_INTERVAL, MAX_BACKOFF, UNAVAILABLE_INTERVAL};

const BASE: Duration = DEFAULT_POLL_INTERVAL;

fn ok() -> oxikube_domain::OxiResult<MetricsOutcome> {
    Ok(MetricsOutcome::Available(Vec::new()))
}

fn failed() -> oxikube_domain::OxiResult<MetricsOutcome> {
    Err(OxiError::network("connection refused"))
}

fn absent(reason: MissingReason) -> oxikube_domain::OxiResult<MetricsOutcome> {
    Ok(MetricsOutcome::Unavailable(reason))
}

#[test]
fn a_kind_never_polled_is_due_at_once_and_then_every_fifteen_seconds() {
    let mut cadence = Cadence::default();
    assert_eq!(BASE, Duration::from_secs(15));
    assert!(cadence.is_due(BASE, t0()));
    cadence.record(t0(), &ok());
    assert_eq!(cadence.delay(BASE), BASE);
    assert!(!cadence.is_due(BASE, at(14)));
    assert!(cadence.is_due(BASE, at(15)));
    assert_eq!(cadence.due(BASE), Some(at(15)));
}

#[test]
fn errors_back_off_doubling_up_to_the_cap_and_an_answer_resets() {
    let mut cadence = Cadence::default();
    let mut delays = Vec::new();
    for _ in 0..5 {
        cadence.record(t0(), &failed());
        delays.push(cadence.delay(BASE).as_secs());
    }
    assert_eq!(delays, [30, 60, 120, 120, 120]);
    assert_eq!(MAX_BACKOFF, Duration::from_secs(120));
    assert_eq!(cadence.failures(), 5);

    cadence.record(t0(), &ok());
    assert_eq!(cadence.delay(BASE), BASE, "an answer resets the backoff");
    assert_eq!(cadence.failures(), 0);
}

#[test]
fn a_503_backs_off_like_an_error() {
    let mut cadence = Cadence::default();
    cadence.record(t0(), &absent(MissingReason::ApiUnavailable));
    assert_eq!(cadence.delay(BASE), Duration::from_secs(30));
}

#[test]
fn an_absent_or_forbidden_api_is_asked_rarely() {
    for reason in [MissingReason::NotInstalled, MissingReason::Forbidden] {
        let mut cadence = Cadence::default();
        cadence.record(t0(), &absent(reason));
        assert_eq!(cadence.delay(BASE), UNAVAILABLE_INTERVAL, "{reason:?}");
        assert_eq!(UNAVAILABLE_INTERVAL, Duration::from_secs(300));
        cadence.record(t0(), &ok());
        assert_eq!(cadence.delay(BASE), BASE, "{reason:?} then installed");
    }
}

#[test]
fn a_long_interval_is_never_shortened_by_the_caps() {
    let base = Duration::from_secs(600);
    let mut cadence = Cadence::default();
    cadence.record(t0(), &failed());
    assert_eq!(cadence.delay(base), base);
    cadence.record(t0(), &absent(MissingReason::NotInstalled));
    assert_eq!(cadence.delay(base), base);
}

#[test]
fn a_resumed_poller_waits_at_most_one_interval_until_the_next_poll() {
    let mut cadence = Cadence::default();
    cadence.record(t0(), &absent(MissingReason::NotInstalled));
    cadence.resume();
    assert_eq!(cadence.delay(BASE), BASE);
    assert!(cadence.is_due(BASE, at(15)));
    // The poll after the resume restores the slow cadence.
    cadence.record(at(15), &absent(MissingReason::NotInstalled));
    assert_eq!(cadence.delay(BASE), UNAVAILABLE_INTERVAL);
}
