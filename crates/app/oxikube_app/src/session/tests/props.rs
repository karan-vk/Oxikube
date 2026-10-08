//! Property test: random interleavings of every manager operation (including held and
//! dropped connects) never reach an illegal state, and the update stream tells the truth.

use std::collections::HashMap;

use futures::FutureExt;
use futures::future::BoxFuture;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::{ClusterSessionState, SessionEventKind, SessionPhase};
use oxikube_domain::{Capabilities, OxiError, OxiResult};
use oxikube_ports::HealthSignal;
use proptest::prelude::*;

use super::{Harness, ctx, id};
use crate::session::{SessionChange, SessionOptions};

const NAMES: [&str; 2] = ["a", "b"];

#[derive(Debug, Clone)]
enum Outcome {
    Ok,
    Auth,
    Fatal,
    DiscoveryAuth,
}

#[derive(Debug, Clone)]
enum Op {
    Connect(usize, Outcome),
    Reconnect(usize, Outcome),
    Disconnect(usize),
    Close(usize),
    Open(usize),
    Health(usize, u8),
    ReportHealth(usize, u8),
    ReadOnly(usize, bool),
    Hold,
    Release,
    DropPending,
}

fn outcome() -> impl Strategy<Value = Outcome> {
    prop_oneof![
        3 => Just(Outcome::Ok),
        1 => Just(Outcome::Auth),
        1 => Just(Outcome::Fatal),
        1 => Just(Outcome::DiscoveryAuth),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    let c = 0..NAMES.len();
    prop_oneof![
        4 => (c.clone(), outcome()).prop_map(|(c, o)| Op::Connect(c, o)),
        2 => (c.clone(), outcome()).prop_map(|(c, o)| Op::Reconnect(c, o)),
        2 => c.clone().prop_map(Op::Disconnect),
        1 => c.clone().prop_map(Op::Close),
        1 => c.clone().prop_map(Op::Open),
        3 => (c.clone(), 0u8..3).prop_map(|(c, s)| Op::Health(c, s)),
        1 => (c.clone(), 0u8..3).prop_map(|(c, s)| Op::ReportHealth(c, s)),
        1 => (c, any::<bool>()).prop_map(|(c, r)| Op::ReadOnly(c, r)),
        1 => Just(Op::Hold),
        2 => Just(Op::Release),
        1 => Just(Op::DropPending),
    ]
}

fn signal(n: u8) -> HealthSignal {
    match n {
        0 => HealthSignal::Healthy,
        1 => HealthSignal::Unhealthy,
        _ => HealthSignal::Failed {
            reason: "probe gave up".into(),
            kind: oxikube_domain::ErrorKind::Timeout,
            retryable: true,
        },
    }
}

/// Scripts the outcome of `cluster`'s next connect, replacing an earlier one nothing
/// consumed. Scripts are per cluster and read when the connector call gets past the hold,
/// so a held connect fails (or not) as the latest op on its cluster said.
fn script(h: &Harness, cluster: &ClusterId, outcome: &Outcome) {
    clear_scripts(h, cluster);
    let connect = h.connector.connect_script_for(cluster);
    match outcome {
        Outcome::Ok => {}
        Outcome::Auth => {
            connect.push_err(OxiError::auth("login", false));
        }
        Outcome::Fatal => {
            connect.push_err(OxiError::internal("boom"));
        }
        Outcome::DiscoveryAuth => {
            h.connector
                .ports_for(cluster)
                .discovery
                .script()
                .discover
                .push_err(OxiError::auth("401", true));
        }
    }
}

fn clear_scripts(h: &Harness, cluster: &ClusterId) {
    h.connector.connect_script_for(cluster).clear();
    h.connector
        .ports_for(cluster)
        .discovery
        .script()
        .discover
        .clear();
}

type Pending<'a> = Vec<(usize, BoxFuture<'a, OxiResult<ClusterSessionState>>)>;

fn poll_all(pending: &mut Pending<'_>) {
    pending.retain_mut(|(_, fut)| fut.now_or_never().is_none());
}

/// Whether some event of the domain table moves `from` to `to`.
fn legal(from: SessionPhase, to: SessionPhase) -> bool {
    SessionEventKind::ALL
        .iter()
        .any(|kind| from.next(*kind) == Some(to))
}

fn check(
    h: &mut Harness,
    ids: &[ClusterId],
    pending: &Pending<'_>,
    seen: &mut HashMap<ClusterId, SessionPhase>,
) {
    for update in h.drain() {
        match update.change {
            SessionChange::Opened => {
                assert!(!seen.contains_key(&update.cluster), "opened twice");
                seen.insert(update.cluster, SessionPhase::Disconnected);
            }
            SessionChange::Closed => {
                assert_eq!(
                    seen.remove(&update.cluster),
                    Some(SessionPhase::Disconnected)
                );
            }
            SessionChange::StateChanged { from, state } => {
                let last = seen
                    .get_mut(&update.cluster)
                    .expect("state of an unopened session");
                assert_eq!(*last, from, "update stream skipped a state");
                assert!(legal(from, state.phase()), "{from} -> {}", state.phase());
                assert_ne!(from, state.phase(), "self-transition announced");
                *last = state.phase();
            }
            _ => {}
        }
    }
    for (i, cluster) in ids.iter().enumerate() {
        let live = h.connector.live_connections(cluster);
        let Some(session) = h.manager.get(cluster) else {
            assert!(!seen.contains_key(cluster));
            assert_eq!(live, 0, "closed session kept a connection");
            continue;
        };
        assert_eq!(
            seen.get(cluster),
            Some(&session.phase()),
            "stream and state disagree"
        );
        let connected = session.is_connected();
        assert_eq!(session.resources().is_some(), connected);
        assert_eq!(live, usize::from(connected), "connection count");
        if !connected {
            assert_eq!(session.capabilities(), Capabilities::empty());
        }
        if session.phase() == SessionPhase::Connecting {
            assert!(pending.iter().any(|(c, _)| *c == i), "stuck in Connecting");
        }
    }
}

/// Applies `ops` in order, checking every invariant after each, then releases and
/// drains whatever is still in flight. Returns each cluster's final phase (`None` once
/// closed).
fn run(ops: &[Op]) -> Vec<Option<SessionPhase>> {
    let mut h = Harness::new();
    let ids: Vec<ClusterId> = NAMES.iter().map(|n| id(n)).collect();
    let manager = h.manager.clone();
    let mut pending: Pending<'_> = Vec::new();
    let mut seen = HashMap::new();
    for op in ops {
        match op {
            Op::Connect(c, o) | Op::Reconnect(c, o) => {
                script(&h, &ids[*c], o);
                let mut fut = if matches!(op, Op::Connect(..)) {
                    manager.connect(&ids[*c]).boxed()
                } else {
                    manager.reconnect(&ids[*c]).boxed()
                };
                if (&mut fut).now_or_never().is_none() {
                    pending.push((*c, fut));
                }
            }
            Op::Disconnect(c) => {
                let _ = manager.disconnect(&ids[*c]);
            }
            Op::Close(c) => {
                manager.close(&ids[*c]);
            }
            Op::Open(c) => {
                manager.open(&ctx(NAMES[*c]), SessionOptions::default());
            }
            Op::Health(c, s) => {
                h.connector.report(&ids[*c], signal(*s));
            }
            Op::ReportHealth(c, s) => {
                manager.report_health(&ids[*c], signal(*s));
            }
            Op::ReadOnly(c, r) => {
                let _ = manager.set_read_only(&ids[*c], *r);
            }
            Op::Hold => h.connector.hold(),
            Op::Release => h.connector.release(),
            Op::DropPending => pending.clear(),
        }
        // Wake whatever an abort or a release unblocked, as an executor would.
        poll_all(&mut pending);
        // A cluster with nothing in flight drops its unconsumed outcome; one with a held
        // connect keeps it for the release.
        for (i, cluster) in ids.iter().enumerate() {
            if !pending.iter().any(|(c, _)| *c == i) {
                clear_scripts(&h, cluster);
            }
        }
        check(&mut h, &ids, &pending, &mut seen);
    }
    h.connector.release();
    poll_all(&mut pending);
    assert!(pending.is_empty(), "a connect never finished");
    check(&mut h, &ids, &pending, &mut seen);
    for session in manager.sessions() {
        assert_ne!(session.phase(), SessionPhase::Connecting);
    }
    ids.iter()
        .map(|cluster| manager.get(cluster).map(|s| s.phase()))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn random_operations_keep_every_invariant(ops in proptest::collection::vec(op(), 1..40)) {
        run(&ops);
    }
}

#[test]
fn a_held_connect_takes_its_scripted_failure_at_release() {
    use Outcome::*;
    for (outcome, phase) in [
        (Auth, SessionPhase::AuthRequired),
        (DiscoveryAuth, SessionPhase::AuthRequired),
        (Fatal, SessionPhase::Error),
        (Ok, SessionPhase::Ready),
    ] {
        let ops = [Op::Hold, Op::Connect(0, outcome.clone()), Op::Release];
        assert_eq!(run(&ops), [Some(phase), None], "{outcome:?}");
    }
    // `b`'s held connect, released first, does not take `a`'s outcome.
    let ops = [
        Op::Hold,
        Op::Connect(1, Ok),
        Op::Connect(0, Fatal),
        Op::Release,
    ];
    assert_eq!(
        run(&ops),
        [Some(SessionPhase::Error), Some(SessionPhase::Ready)]
    );
    // A cancelled held connect's outcome does not leak into the next connect.
    let ops = [
        Op::Hold,
        Op::Connect(0, Auth),
        Op::Disconnect(0),
        Op::Release,
        Op::Connect(0, Ok),
    ];
    assert_eq!(run(&ops), [Some(SessionPhase::Ready), None]);
}
