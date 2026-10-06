//! `Ready` ↔ `Degraded`, `Failed` → `Error`, and stale reports.

use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_domain::{Capabilities, OxiError};
use oxikube_ports::HealthSignal;

use super::{Harness, id};

#[test]
fn ready_degrades_and_recovers() {
    let mut h = Harness::new();
    let a = id("a");
    h.connect("a");
    h.drain();

    assert!(h.connector.report(&a, HealthSignal::Unhealthy));
    assert_eq!(h.phase("a"), SessionPhase::Degraded);
    // Still connected: ports and capabilities stay.
    let session = h.manager.get(&a).unwrap();
    assert!(session.is_connected() && session.resources().is_some());
    assert_eq!(session.capabilities(), Capabilities::all());

    h.connector.report(&a, HealthSignal::Unhealthy);
    h.connector.report(&a, HealthSignal::Healthy);
    h.connector.report(&a, HealthSignal::Healthy);
    // Self-transitions are not announced.
    assert_eq!(h.phases("a"), [SessionPhase::Degraded, SessionPhase::Ready]);
}

#[test]
fn giving_up_moves_to_error_and_drops_the_connection() {
    let mut h = Harness::new();
    let a = id("a");
    h.connect("a");
    h.connector.report(&a, HealthSignal::Unhealthy);
    h.connector.report(
        &a,
        HealthSignal::Failed {
            reason: "3 probes failed".into(),
        },
    );
    let session = h.manager.get(&a).unwrap();
    assert_eq!(
        session.state(),
        &ClusterSessionState::Error {
            reason: "3 probes failed".into()
        }
    );
    assert!(session.resources().is_none());
    assert_eq!(session.capabilities(), Capabilities::empty());
    assert_eq!(h.connector.live_connections(&a), 0);
    // The dead connection's reporter no longer moves the session.
    h.drain();
    h.connector.report(&a, HealthSignal::Healthy);
    assert!(h.drain().is_empty());
}

#[test]
fn reports_of_a_replaced_connection_are_ignored() {
    let mut h = Harness::new();
    let a = id("a");
    h.connect("a");
    // Keep the first connection's reporter, then reconnect.
    let old = h.connector.reporter(&a).expect("reporter");
    h.reconnect("a");
    h.drain();
    old.report(HealthSignal::Unhealthy);
    assert_eq!(h.phase("a"), SessionPhase::Ready);
    assert!(h.drain().is_empty());
    // The current one still works.
    h.connector.report(&a, HealthSignal::Unhealthy);
    assert_eq!(h.phase("a"), SessionPhase::Degraded);
}

#[test]
fn reports_before_ready_or_after_disconnect_are_ignored() {
    let h = Harness::new();
    let a = id("a");
    h.connector
        .script()
        .connect
        .push_err(OxiError::auth("login", false));
    h.connect("a");
    assert!(!h.manager.report_health(&a, HealthSignal::Healthy));
    assert_eq!(h.phase("a"), SessionPhase::AuthRequired);

    h.connect("a");
    h.manager.disconnect(&a).unwrap();
    assert!(h.connector.report(&a, HealthSignal::Unhealthy));
    assert_eq!(h.phase("a"), SessionPhase::Disconnected);
}

#[test]
fn other_services_can_report_feed_health() {
    let h = Harness::new();
    let a = id("a");
    h.connect("a");
    assert!(h.manager.report_health(&a, HealthSignal::Unhealthy));
    assert_eq!(h.phase("a"), SessionPhase::Degraded);
    assert!(h.manager.report_health(&a, HealthSignal::Healthy));
    assert_eq!(h.phase("a"), SessionPhase::Ready);
    assert!(!h.manager.report_health(&id("b"), HealthSignal::Healthy));
}

/// A connector whose connection teardown reports health, as a liveness loop stopping on
/// drop might: releasing a connection must not hold the session lock.
struct ReportsOnTeardown;

struct Teardown(std::sync::Arc<dyn oxikube_ports::HealthReporter>);

impl Drop for Teardown {
    fn drop(&mut self) {
        self.0.report(HealthSignal::Unhealthy);
    }
}

#[async_trait::async_trait]
impl oxikube_ports::ClusterConnectorPort for ReportsOnTeardown {
    async fn connect(
        &self,
        request: oxikube_ports::ConnectRequest,
    ) -> oxikube_domain::OxiResult<oxikube_ports::ClusterConnection> {
        Ok(oxikube_ports::ClusterConnection {
            ports: oxikube_testkit::FakeClusterPorts::default().ports(),
            guard: oxikube_ports::ConnectionGuard::new(Teardown(request.health)),
        })
    }
}

#[test]
fn teardown_that_reports_health_does_not_deadlock() {
    use futures::FutureExt;
    let h = Harness::new();
    let manager = crate::session::ClusterSessionManager::new(
        std::sync::Arc::new(ReportsOnTeardown),
        h.source.clone(),
        h.clock,
    );
    let a = id("a");
    let connect =
        |m: &crate::session::ClusterSessionManager| m.connect(&a).now_or_never().unwrap().unwrap();
    assert_eq!(connect(&manager), ClusterSessionState::Ready);
    manager.disconnect(&a).unwrap();
    assert_eq!(connect(&manager), ClusterSessionState::Ready);
    manager.reconnect(&a).now_or_never().unwrap().unwrap();
    manager.report_health(
        &a,
        HealthSignal::Failed {
            reason: "gone".into(),
        },
    );
    assert_eq!(manager.get(&a).unwrap().phase(), SessionPhase::Error);
    assert!(manager.close(&a));
}
