//! The restart state machine: every transition of the diagram, the backoff and its jitter bounds,
//! permanent against transient errors.

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::time::Duration;

use oxikube_domain::notification::ForwardPhase;
use oxikube_ports::PortForwardError;

use crate::portforward::state::{
    Backoff, ForwardEvent as E, ForwardFailure, ForwardState as S, is_permanent, step,
};

fn addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 18080))
}

fn active() -> S {
    S::Active {
        pod: "web-0".into(),
        local_addr: addr(),
    }
}

fn reconnecting(attempt: u32) -> S {
    S::Reconnecting {
        attempt,
        delay: Duration::ZERO,
        reason: "x".into(),
    }
}

fn connected() -> E {
    E::Connected {
        pod: "web-0".into(),
        local_addr: addr(),
    }
}

fn gone() -> E {
    E::Lost(PortForwardError::PodGone("shop/web-0".into()))
}

fn forbidden() -> E {
    E::Lost(PortForwardError::Forbidden("pods/portforward".into()))
}

fn changed() -> E {
    E::TargetChanged {
        reason: "replaced".into(),
    }
}

fn failed() -> S {
    S::Failed(ForwardFailure::message("x"))
}

/// `step` with the default backoff and no jitter.
fn next(state: &S, event: E) -> S {
    step(state, event, &Backoff::default(), 0.5)
}

#[test]
fn every_transition_of_the_diagram() {
    // (from, event, expected phase)
    let table: Vec<(S, E, ForwardPhase)> = vec![
        (S::Starting, connected(), ForwardPhase::Active),
        (S::Starting, forbidden(), ForwardPhase::Failed),
        (S::Starting, gone(), ForwardPhase::Reconnecting),
        (S::Starting, E::Stop, ForwardPhase::Stopped),
        (active(), gone(), ForwardPhase::Reconnecting),
        (active(), changed(), ForwardPhase::Reconnecting),
        (active(), forbidden(), ForwardPhase::Failed),
        (active(), E::Stop, ForwardPhase::Stopped),
        (reconnecting(1), connected(), ForwardPhase::Active),
        (reconnecting(1), gone(), ForwardPhase::Reconnecting),
        (reconnecting(1), changed(), ForwardPhase::Reconnecting),
        (reconnecting(1), forbidden(), ForwardPhase::Failed),
        (reconnecting(12), gone(), ForwardPhase::Failed),
        (reconnecting(1), E::Stop, ForwardPhase::Stopped),
        (failed(), E::Restart, ForwardPhase::Starting),
        (S::Stopped, E::Restart, ForwardPhase::Starting),
        (active(), E::Restart, ForwardPhase::Starting),
        // Terminal states ignore everything but a restart.
        (failed(), connected(), ForwardPhase::Failed),
        (S::Stopped, gone(), ForwardPhase::Stopped),
        (S::Stopped, E::Stop, ForwardPhase::Stopped),
    ];
    for (from, event, phase) in table {
        let label = format!("{from:?} + {event:?}");
        assert_eq!(next(&from, event).phase(), phase, "{label}");
    }
}

#[test]
fn a_drop_retries_at_once_then_backs_off_and_gives_up() {
    let backoff = Backoff::default();
    let mut state = next(&active(), gone());
    assert!(
        matches!(state, S::Reconnecting { attempt: 1, delay, .. } if delay.is_zero()),
        "the first reconnect is immediate: {state:?}"
    );
    let mut delays = Vec::new();
    for _ in 1..backoff.max_attempts {
        state = next(&state, gone());
        let S::Reconnecting { delay, .. } = &state else {
            panic!("still reconnecting: {state:?}");
        };
        delays.push(*delay);
    }
    assert_eq!(delays[0], Duration::from_millis(500));
    assert_eq!(delays[1], Duration::from_secs(1));
    assert_eq!(delays[2], Duration::from_secs(2));
    assert_eq!(*delays.last().unwrap(), Duration::from_secs(15), "capped");
    assert!(delays.windows(2).all(|w| w[0] <= w[1]), "never shrinks");
    let S::Failed(failure) = next(&state, gone()) else {
        panic!("gives up after the last attempt");
    };
    assert!(
        failure.message.contains("gave up after 12 attempts"),
        "{}",
        failure.message
    );
}

#[test]
fn a_watch_event_retries_now_without_counting_an_attempt() {
    let waiting = S::Reconnecting {
        attempt: 4,
        delay: Duration::from_secs(4),
        reason: "no ready pod".into(),
    };
    assert!(matches!(
        next(&waiting, changed()),
        S::Reconnecting { attempt: 4, delay, .. } if delay.is_zero()
    ));
    assert_eq!(next(&S::Starting, changed()), S::Starting);
}

#[test]
fn unresolvable_targets_wait_unless_permanent() {
    let waiting = next(
        &S::Starting,
        E::Unresolvable {
            reason: "no ready pod".into(),
            permanent: false,
        },
    );
    assert_eq!(waiting.reason(), Some("no ready pod"));
    assert_eq!(waiting.phase(), ForwardPhase::Reconnecting);
    let invalid = next(
        &S::Starting,
        E::Unresolvable {
            reason: "no port http".into(),
            permanent: true,
        },
    );
    assert_eq!(invalid.phase(), ForwardPhase::Failed);
}

#[test]
fn jitter_stays_within_its_bounds_and_the_cap() {
    let backoff = Backoff::default();
    for attempt in 2..20 {
        let base = backoff.delay(attempt, 0.5);
        for unit in [0.0, 0.1, 0.37, 0.5, 0.9, 0.999] {
            let d = backoff.delay(attempt, unit);
            assert!(d <= backoff.cap, "attempt {attempt} unit {unit}: {d:?}");
            assert!(
                d >= base.mul_f64(0.8),
                "attempt {attempt} unit {unit}: {d:?}"
            );
            assert!(
                d <= base.mul_f64(1.2),
                "attempt {attempt} unit {unit}: {d:?}"
            );
        }
    }
    assert!(
        backoff.delay(1, 0.99).is_zero(),
        "the first attempt never waits"
    );
    assert_ne!(
        backoff.delay(3, 0.0),
        backoff.delay(3, 0.99),
        "jitter spreads"
    );
}

#[test]
fn permanent_and_transient_errors() {
    let busy = PortForwardError::BindFailed {
        addr: addr(),
        kind: ErrorKind::AddrInUse,
    };
    let privileged = PortForwardError::BindFailed {
        addr: SocketAddr::from(([127, 0, 0, 1], 80)),
        kind: ErrorKind::PermissionDenied,
    };
    // A busy port at the start is the user's to fix; while reconnecting it is the old listener.
    assert!(is_permanent(&busy, &S::Starting));
    assert!(!is_permanent(&busy, &reconnecting(1)));
    assert!(is_permanent(&privileged, &reconnecting(1)));
    assert!(is_permanent(
        &PortForwardError::Forbidden("x".into()),
        &reconnecting(1)
    ));
    for transient in [
        PortForwardError::PodGone("p".into()),
        PortForwardError::PodNotReady("p".into()),
        PortForwardError::Disconnected("x".into()),
    ] {
        assert!(!is_permanent(&transient, &S::Starting), "{transient:?}");
    }
    let S::Failed(failure) = next(&S::Starting, E::Lost(privileged)) else {
        panic!("a privileged port fails");
    };
    assert!(failure.suggestion().unwrap().contains("1024"));
}
