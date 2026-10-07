//! Automatic reconnect after a health `Failed` (E06-F440), on a current-thread Tokio runtime
//! (the schedule is a task) and the virtual clock.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    ClockPort as _, ClusterConnection, ClusterConnectorPort, ConnectRequest, HealthSignal,
};
use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort};
use parking_lot::Mutex;

use super::{Harness, id};
use crate::session::{AutoReconnect, ClusterSessionManager, RetryPolicy, SessionManagerConfig};

/// The fake connector behind a network outage: every connect fails with a retryable `Network`
/// error until the virtual clock reaches the outage's end, then the fake connects as usual.
struct Outage {
    inner: Arc<FakeClusterConnectorPort>,
    clock: Arc<FakeClockPort>,
    until: Mutex<Option<Timestamp>>,
    failed: Mutex<u32>,
}

#[async_trait]
impl ClusterConnectorPort for Outage {
    async fn connect(&self, request: ConnectRequest) -> OxiResult<ClusterConnection> {
        let down = (*self.until.lock()).is_some_and(|until| self.clock.now() < until);
        if down {
            *self.failed.lock() += 1;
            return Err(OxiError::network("connection refused"));
        }
        self.inner.connect(request).await
    }
}

/// A manager over [`Outage`]. Connects inside an attempt are not retried, so each automatic
/// attempt is one connect and the timeline is the reconnect backoff alone.
struct Fixture {
    manager: ClusterSessionManager,
    connector: Arc<FakeClusterConnectorPort>,
    outage: Arc<Outage>,
    clock: Arc<FakeClockPort>,
    start: Timestamp,
}

impl Fixture {
    fn new() -> Self {
        Self::with_policy(RetryPolicy::auto_reconnect())
    }

    fn with_policy(auto_reconnect: RetryPolicy) -> Self {
        let h = Harness::new();
        let outage = Arc::new(Outage {
            inner: h.connector.clone(),
            clock: h.clock.clone(),
            until: Mutex::new(None),
            failed: Mutex::new(0),
        });
        let manager = ClusterSessionManager::with_config(
            outage.clone(),
            h.source.clone(),
            h.clock.clone(),
            SessionManagerConfig {
                retry: RetryPolicy::no_retry(),
                auto_reconnect: Some(auto_reconnect),
                ..SessionManagerConfig::default()
            },
        );
        let start = h.clock.now();
        Self {
            manager,
            connector: h.connector,
            outage,
            clock: h.clock,
            start,
        }
    }

    /// Connects `a` and then cuts the network for `outage` from now.
    async fn connected_then_outage(&self, outage: Duration) {
        let state = self.manager.connect(&id("a")).await.unwrap();
        assert_eq!(state, ClusterSessionState::Ready);
        *self.outage.until.lock() = Some(self.clock.now() + outage);
    }

    /// The liveness loop gives up with `error`, as the adapter reports it.
    fn liveness_fails(&self, error: &OxiError) {
        let a = id("a");
        assert!(self.connector.report(&a, HealthSignal::Unhealthy));
        assert!(self.connector.report(&a, HealthSignal::failed(error)));
    }

    fn phase(&self) -> SessionPhase {
        self.manager.get(&id("a")).unwrap().phase()
    }

    fn plan(&self) -> Option<AutoReconnect> {
        self.manager.get(&id("a")).unwrap().auto_reconnect()
    }

    fn elapsed(&self) -> Duration {
        Duration::try_from(self.clock.now().duration_since(self.start)).unwrap()
    }

    /// Lets the spawned tasks run until they wait for the clock again.
    async fn settle(&self) {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    /// Runs virtual time from sleeper to sleeper until nothing sleeps any more.
    async fn run_until_idle(&self) {
        self.settle().await;
        while self.clock.advance_to_next().is_some() {
            self.settle().await;
        }
    }
}

fn plan(attempt: u32, secs: u64) -> Option<AutoReconnect> {
    Some(AutoReconnect {
        attempt,
        delay: Duration::from_secs(secs),
    })
}

#[tokio::test]
async fn a_network_outage_is_ridden_out_with_backoff_and_ends_ready() {
    let fx = Fixture::new();
    // The network is down for 10 s from the moment the probes give up.
    fx.connected_then_outage(Duration::from_secs(10)).await;
    fx.liveness_fails(&OxiError::network("connection reset"));
    assert_eq!(fx.phase(), SessionPhase::Error);
    assert_eq!(
        fx.plan(),
        plan(1, 1),
        "the error says a reconnect is coming"
    );
    assert_eq!(fx.connector.live_connections(&id("a")), 0);

    // Attempts at 1 s, 3 s and 7 s fail; the one at 15 s finds the network back.
    fx.run_until_idle().await;
    assert_eq!(fx.phase(), SessionPhase::Ready);
    assert_eq!(*fx.outage.failed.lock(), 3);
    assert_eq!(fx.elapsed(), Duration::from_secs(15));
    assert_eq!(fx.plan(), None, "the schedule ends at Ready");
    assert_eq!(fx.connector.live_connections(&id("a")), 1);

    // The new connection's health is followed: its liveness loop runs again.
    assert!(fx.connector.report(&id("a"), HealthSignal::Unhealthy));
    assert_eq!(fx.phase(), SessionPhase::Degraded);
}

#[tokio::test]
async fn each_failed_attempt_is_shown_with_the_next_delay() {
    let fx = Fixture::new();
    fx.connected_then_outage(Duration::from_secs(60)).await;
    fx.liveness_fails(&OxiError::timeout("the cluster did not answer within 10s"));
    let mut seen = vec![fx.plan()];
    fx.settle().await;
    for _ in 0..3 {
        fx.clock.advance_to_next();
        fx.settle().await;
        assert_eq!(fx.phase(), SessionPhase::Error);
        seen.push(fx.plan());
    }
    assert_eq!(seen, [plan(1, 1), plan(2, 2), plan(3, 4), plan(4, 8)]);
}

#[tokio::test]
async fn a_retryable_auth_failure_reconnects_too() {
    let fx = Fixture::new();
    fx.connected_then_outage(Duration::ZERO).await;
    fx.liveness_fails(&OxiError::auth("token expired", true));
    assert_eq!(fx.plan(), plan(1, 1));
    fx.run_until_idle().await;
    assert_eq!(fx.phase(), SessionPhase::Ready);
}

#[tokio::test]
async fn revoked_credentials_ask_for_them_and_are_not_retried() {
    let fx = Fixture::new();
    fx.connected_then_outage(Duration::ZERO).await;
    fx.liveness_fails(&OxiError::auth("Unauthorized: token revoked", false));
    let session = fx.manager.get(&id("a")).unwrap();
    assert_eq!(
        session.state(),
        &ClusterSessionState::AuthRequired {
            reason: "Unauthorized: token revoked".into()
        }
    );
    assert!(session.resources().is_none(), "the connection is released");
    assert_eq!(fx.plan(), None);
    fx.settle().await;
    assert_eq!(fx.clock.pending_sleepers(), 0, "nothing is scheduled");
}

#[tokio::test]
async fn permanent_failures_stay_in_error() {
    let permanent = [
        OxiError::network("certificate has expired").with_retryable(false),
        OxiError::forbidden("/version is forbidden"),
        OxiError::internal("bad gateway body"),
    ];
    for error in permanent {
        let fx = Fixture::new();
        fx.connected_then_outage(Duration::ZERO).await;
        fx.liveness_fails(&error);
        assert_eq!(fx.phase(), SessionPhase::Error, "{error}");
        assert_eq!(fx.plan(), None, "{error}");
        fx.settle().await;
        assert_eq!(fx.clock.pending_sleepers(), 0, "{error}");
    }
}

#[tokio::test]
async fn the_schedule_stops_when_the_connect_needs_credentials() {
    let fx = Fixture::new();
    fx.connected_then_outage(Duration::ZERO).await;
    fx.connector
        .script()
        .connect
        .push_err(OxiError::auth("exec plugin needs a login", false));
    fx.liveness_fails(&OxiError::network("reset"));
    fx.run_until_idle().await;
    assert_eq!(fx.phase(), SessionPhase::AuthRequired);
    assert_eq!(fx.plan(), None);
}

#[tokio::test]
async fn a_connect_failure_that_is_permanent_ends_the_schedule() {
    let fx = Fixture::new();
    fx.connected_then_outage(Duration::ZERO).await;
    fx.connector
        .script()
        .connect
        .push_err(OxiError::not_found("context left the kubeconfig"));
    fx.liveness_fails(&OxiError::network("reset"));
    fx.run_until_idle().await;
    assert_eq!(fx.phase(), SessionPhase::Error);
    assert_eq!(fx.plan(), None);
    assert_eq!(fx.elapsed(), Duration::from_secs(1), "one attempt only");
}

#[tokio::test]
async fn the_attempts_can_run_out() {
    let fx = Fixture::with_policy(RetryPolicy {
        max_attempts: 2,
        ..RetryPolicy::auto_reconnect()
    });
    fx.connected_then_outage(Duration::from_secs(3600)).await;
    fx.liveness_fails(&OxiError::network("reset"));
    fx.run_until_idle().await;
    assert_eq!(fx.phase(), SessionPhase::Error);
    assert_eq!(*fx.outage.failed.lock(), 2);
    assert_eq!(fx.plan(), None, "the error no longer promises a reconnect");
}

#[tokio::test]
async fn disconnecting_or_closing_cancels_the_schedule() {
    let fx = Fixture::new();
    fx.connected_then_outage(Duration::ZERO).await;
    fx.liveness_fails(&OxiError::network("reset"));
    fx.settle().await;
    assert_eq!(fx.clock.pending_sleepers(), 1);

    fx.manager.disconnect(&id("a")).unwrap();
    assert_eq!(fx.plan(), None);
    fx.settle().await;
    assert_eq!(fx.clock.pending_sleepers(), 0, "the task was aborted");
    fx.run_until_idle().await;
    assert_eq!(fx.phase(), SessionPhase::Disconnected);

    // Closing a session in the middle of a schedule ends it as well.
    fx.manager.connect(&id("a")).await.unwrap();
    fx.liveness_fails(&OxiError::network("reset"));
    assert!(fx.manager.close(&id("a")));
    fx.settle().await;
    assert_eq!(fx.clock.pending_sleepers(), 0);
}

#[tokio::test]
async fn a_retry_by_the_user_takes_over_from_the_schedule() {
    let fx = Fixture::new();
    fx.connected_then_outage(Duration::from_secs(3600)).await;
    fx.liveness_fails(&OxiError::network("reset"));
    fx.settle().await;

    // The user presses Retry while the network is still down: their attempt fails, and the
    // session waits for them again instead of reconnecting behind their back.
    let state = fx.manager.connect(&id("a")).await.unwrap();
    assert_eq!(state.phase(), SessionPhase::Error);
    assert_eq!(fx.plan(), None);
    fx.run_until_idle().await;
    assert_eq!(*fx.outage.failed.lock(), 1, "only the user's attempt ran");
}

#[tokio::test]
async fn a_failure_after_the_reconnect_starts_a_fresh_schedule() {
    let fx = Fixture::new();
    fx.connected_then_outage(Duration::from_secs(2)).await;
    fx.liveness_fails(&OxiError::network("reset"));
    fx.run_until_idle().await;
    assert_eq!(fx.phase(), SessionPhase::Ready);

    // The new connection fails as well: the backoff starts over at attempt 1.
    fx.liveness_fails(&OxiError::network("reset again"));
    assert_eq!(fx.plan(), plan(1, 1));
    fx.run_until_idle().await;
    assert_eq!(fx.phase(), SessionPhase::Ready);
}

#[tokio::test]
async fn automatic_reconnects_can_be_turned_off() {
    let h = Harness::with_config(SessionManagerConfig {
        auto_reconnect: None,
        ..SessionManagerConfig::default()
    });
    let a = id("a");
    h.manager.connect(&a).await.unwrap();
    h.connector
        .report(&a, HealthSignal::failed(&OxiError::network("reset")));
    assert_eq!(h.phase("a"), SessionPhase::Error);
    assert_eq!(h.manager.get(&a).unwrap().auto_reconnect(), None);
    assert_eq!(h.clock.pending_sleepers(), 0);
}

#[test]
fn without_a_runtime_nothing_is_scheduled() {
    let h = Harness::new();
    let a = id("a");
    h.connect("a");
    h.connector
        .report(&a, HealthSignal::failed(&OxiError::network("reset")));
    assert_eq!(h.phase("a"), SessionPhase::Error);
    assert_eq!(h.manager.get(&a).unwrap().auto_reconnect(), None);
}
