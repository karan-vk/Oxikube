//! The service against fake sessions and a scripted `FakeMetricsPort`, timed by a manual clock:
//! the pollers are tasks on the Tokio test runtime and every wait is on a condition with a
//! deadline (the scripted calls, the clock's sleepers), never a sleep before an assertion.

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::OxiError;
use oxikube_domain::metrics::MissingReason;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{ClusterContext, MetricsOutcome, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeMetricsPort, MetricsCall,
};

use super::{cluster, node_sample, pod_sample, t0};
use crate::columns::{Threshold, UsageThresholds};
use crate::metrics::live::{
    LiveMetricsConfig, LiveMetricsConfigTable, MetricsKind, MetricsService, MetricsState,
};
use crate::session::ClusterSessionManager;
use crate::store::Spawner;

struct Harness {
    sessions: ClusterSessionManager,
    connector: Arc<FakeClusterConnectorPort>,
    clock: Arc<FakeClockPort>,
    service: MetricsService,
}

fn tokio_spawner() -> Arc<dyn Spawner> {
    Arc::new(|task: futures::future::BoxFuture<'static, ()>| {
        tokio::spawn(task);
    })
}

impl Harness {
    fn new() -> Self {
        let context = oxikube_domain::ids::ContextName::new("a");
        let entry = ClusterContext::new(cluster(), context, SourceId("kubeconfig".into()));
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
        // The sessions keep their own clock: only the pollers sleep on `clock`.
        let sessions = ClusterSessionManager::new(
            connector.clone(),
            source,
            Arc::new(FakeClockPort::default()),
        );
        let clock = Arc::new(FakeClockPort::new(t0()));
        let service = MetricsService::new(sessions.clone(), clock.clone());
        service.start(tokio_spawner());
        Self {
            sessions,
            connector,
            clock,
            service,
        }
    }

    fn port(&self) -> Arc<FakeMetricsPort> {
        self.connector.ports_for(&cluster()).metrics
    }

    async fn connect(&self) {
        self.sessions.connect(&cluster()).await.expect("connect");
    }

    fn calls(&self) -> Vec<MetricsCall> {
        self.port().recorded_calls()
    }

    /// Waits until `n` calls were made and the poller is asleep again (or stopped).
    async fn settle_at(&self, n: usize) {
        self.until(|h| h.calls().len() >= n).await;
        assert_eq!(self.calls().len(), n, "exactly {n} calls");
    }

    async fn until(&self, done: impl Fn(&Self) -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !done(self) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the condition held in time");
    }

    /// Lets every runnable task run (the poller reaching its sleep).
    async fn idle(&self) {
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
    }

    fn advance(&self, seconds: u64) {
        self.clock.advance(Duration::from_secs(seconds));
    }

    fn script_nodes(&self, n: usize) {
        for _ in 0..n {
            self.port()
                .script()
                .node_metrics
                .push_ok(MetricsOutcome::Available(vec![node_sample(
                    "n1",
                    t0(),
                    100,
                    100,
                )]));
        }
    }
}

#[tokio::test]
async fn a_watched_shown_cluster_is_polled_every_fifteen_seconds() {
    let h = Harness::new();
    h.connect().await;
    h.script_nodes(3);
    let mut window = h.service.visibility();
    window.show(Some(cluster()));
    let _table = h.service.watch(&cluster(), MetricsKind::Nodes);
    h.settle_at(1).await;
    assert_eq!(h.calls(), [MetricsCall::NodeMetrics(cluster())]);
    h.until(|h| {
        h.service
            .state(&cluster(), MetricsKind::Nodes)
            .is_available()
    })
    .await;
    assert!(h.service.usage(&cluster()).node("n1").is_some());

    h.idle().await;
    h.advance(14);
    h.idle().await;
    assert_eq!(h.calls().len(), 1, "not before 15 s");
    h.advance(1);
    h.settle_at(2).await;
    h.idle().await;
    h.advance(15);
    h.settle_at(3).await;
    assert!(
        h.calls()
            .iter()
            .all(|c| matches!(c, MetricsCall::NodeMetrics(_))),
        "only the watched kind is polled"
    );
}

#[tokio::test]
async fn nothing_is_polled_without_a_consumer_or_with_the_tab_hidden() {
    let h = Harness::new();
    h.connect().await;
    h.script_nodes(4);

    // Shown but nobody watching.
    let mut window = h.service.visibility();
    window.show(Some(cluster()));
    h.idle().await;
    h.advance(60);
    h.idle().await;
    assert!(h.calls().is_empty());
    assert!(!h.service.is_polling(&cluster()));

    // Watched: polls; the last consumer gone: stops.
    let table = h.service.watch(&cluster(), MetricsKind::Nodes);
    h.settle_at(1).await;
    drop(table);
    assert!(!h.service.is_polling(&cluster()));
    h.advance(60);
    h.idle().await;
    assert_eq!(
        h.calls().len(),
        1,
        "the poller stopped with its last consumer"
    );

    // Watched again but the tab hidden: silent.
    let _table = h.service.watch(&cluster(), MetricsKind::Nodes);
    window.show(None);
    h.advance(60);
    h.idle().await;
    assert_eq!(h.calls().len(), 1, "a hidden tab is not polled");
    assert!(!h.service.is_polling(&cluster()));

    // Shown again: resumes at once (the last poll is older than one interval).
    window.show(Some(cluster()));
    h.settle_at(2).await;
    // Dropping the window's handle hides it too.
    drop(window);
    assert!(!h.service.is_polling(&cluster()));
}

#[tokio::test]
async fn showing_again_within_an_interval_waits_for_it() {
    let h = Harness::new();
    h.connect().await;
    h.script_nodes(2);
    let mut window = h.service.visibility();
    window.show(Some(cluster()));
    let _table = h.service.watch(&cluster(), MetricsKind::Nodes);
    h.settle_at(1).await;
    window.show(None);
    h.advance(5);
    window.show(Some(cluster()));
    h.idle().await;
    assert_eq!(h.calls().len(), 1, "polled 5 s ago");
    h.advance(10);
    h.settle_at(2).await;
}

#[tokio::test]
async fn errors_back_off_and_an_answer_resets_the_interval() {
    let h = Harness::new();
    h.connect().await;
    let port = h.port();
    let script = &port.script().node_metrics;
    script.push_err(OxiError::network("refused"));
    script.push_err(OxiError::network("refused"));
    script.push_ok(MetricsOutcome::Available(vec![node_sample(
        "n1",
        t0(),
        1,
        1,
    )]));
    script.push_ok(MetricsOutcome::Available(vec![node_sample(
        "n1",
        t0(),
        1,
        1,
    )]));
    let mut window = h.service.visibility();
    window.show(Some(cluster()));
    let _table = h.service.watch(&cluster(), MetricsKind::Nodes);
    h.settle_at(1).await;
    h.until(|h| {
        h.service
            .state(&cluster(), MetricsKind::Nodes)
            .unavailable_text()
            .is_some_and(|t| t.contains("refused"))
    })
    .await;

    // First failure: 30 s.
    h.idle().await;
    h.advance(29);
    h.idle().await;
    assert_eq!(h.calls().len(), 1);
    h.advance(1);
    h.settle_at(2).await;
    // Second: 60 s.
    h.idle().await;
    h.advance(59);
    h.idle().await;
    assert_eq!(h.calls().len(), 2);
    h.advance(1);
    h.settle_at(3).await;
    // Answered: back to 15 s.
    h.until(|h| {
        h.service
            .state(&cluster(), MetricsKind::Nodes)
            .is_available()
    })
    .await;
    h.idle().await;
    h.advance(15);
    h.settle_at(4).await;
}

#[tokio::test]
async fn an_absent_api_is_a_visible_state_polled_rarely() {
    let h = Harness::new();
    h.connect().await;
    let port = h.port();
    let script = &port.script().pod_metrics;
    script.push_ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled));
    script.push_ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled));
    let mut window = h.service.visibility();
    window.show(Some(cluster()));
    let _table = h.service.watch(&cluster(), MetricsKind::Pods);
    h.settle_at(1).await;
    h.until(|h| {
        h.service.state(&cluster(), MetricsKind::Pods)
            == MetricsState::Unavailable(MissingReason::NotInstalled)
    })
    .await;
    h.idle().await;
    h.advance(299);
    h.idle().await;
    assert_eq!(h.calls().len(), 1, "every five minutes, not every 15 s");
    h.advance(1);
    h.settle_at(2).await;
}

#[tokio::test]
async fn pods_are_read_in_the_sessions_namespaces_and_again_when_they_change() {
    let h = Harness::new();
    h.connect().await;
    let port = h.port();
    let script = &port.script().pod_metrics;
    script.push_ok(MetricsOutcome::Available(vec![pod_sample(
        "shop",
        "web",
        t0(),
        1,
        1,
    )]));
    script.push_ok(MetricsOutcome::Available(vec![pod_sample(
        "kube-system",
        "dns",
        t0(),
        1,
        1,
    )]));
    let mut window = h.service.visibility();
    window.show(Some(cluster()));
    let _table = h.service.watch(&cluster(), MetricsKind::Pods);
    h.settle_at(1).await;
    h.until(|h| h.service.usage(&cluster()).pod("shop", "web").is_some())
        .await;

    let one = NamespaceSelection::single("kube-system");
    h.sessions
        .set_namespace_selection(&cluster(), one.clone())
        .expect("select");
    h.settle_at(2).await;
    assert_eq!(h.calls()[1], MetricsCall::PodMetrics(cluster(), one));
    h.until(|h| {
        h.service
            .usage(&cluster())
            .pod("kube-system", "dns")
            .is_some()
    })
    .await;
    assert_eq!(h.service.usage(&cluster()).pod("shop", "web"), None);
}

#[tokio::test]
async fn a_disconnect_stops_polling_and_forgets_the_samples() {
    let h = Harness::new();
    h.connect().await;
    h.script_nodes(2);
    let mut window = h.service.visibility();
    window.show(Some(cluster()));
    let _table = h.service.watch(&cluster(), MetricsKind::Nodes);
    h.settle_at(1).await;
    h.until(|h| h.service.usage(&cluster()).node("n1").is_some())
        .await;

    h.sessions.disconnect(&cluster()).expect("disconnect");
    h.until(|h| !h.service.is_polling(&cluster())).await;
    h.until(|h| h.service.usage(&cluster()).node("n1").is_none())
        .await;
    assert_eq!(
        h.service.state(&cluster(), MetricsKind::Nodes),
        MetricsState::Idle
    );
    h.advance(60);
    h.idle().await;
    assert_eq!(h.calls().len(), 1);

    // Reconnected while still watched and shown: polls again.
    h.connect().await;
    h.settle_at(2).await;
}

#[tokio::test]
async fn the_setting_moves_the_interval_and_the_thresholds() {
    let h = Harness::new();
    h.connect().await;
    h.script_nodes(3);
    let mut window = h.service.visibility();
    window.show(Some(cluster()));
    let _table = h.service.watch(&cluster(), MetricsKind::Nodes);
    h.settle_at(1).await;
    h.idle().await;

    let thresholds = UsageThresholds {
        cpu: Threshold::new(50, 60),
        memory: Threshold::default(),
    };
    h.service
        .set_config(LiveMetricsConfigTable::default().with_cluster(
            cluster(),
            LiveMetricsConfig {
                poll_interval: Duration::from_secs(30),
                thresholds,
            },
        ));
    assert_eq!(h.service.usage(&cluster()).thresholds(), thresholds);
    h.idle().await;
    h.advance(15);
    h.idle().await;
    assert_eq!(h.calls().len(), 1, "the new interval is 30 s");
    h.advance(15);
    h.settle_at(2).await;

    // Out-of-range intervals are clamped (5 s at least).
    let fast = LiveMetricsConfigTable::new(LiveMetricsConfig {
        poll_interval: Duration::from_millis(10),
        ..LiveMetricsConfig::default()
    });
    assert_eq!(fast.get(&cluster()).poll_interval, Duration::from_secs(5));
}

#[tokio::test]
async fn a_second_kind_joins_at_once_and_each_keeps_its_own_cadence() {
    let h = Harness::new();
    h.connect().await;
    h.script_nodes(2);
    h.port()
        .script()
        .pod_metrics
        .push_ok(MetricsOutcome::Unavailable(MissingReason::Forbidden));
    let mut window = h.service.visibility();
    window.show(Some(cluster()));
    let _nodes = h.service.watch(&cluster(), MetricsKind::Nodes);
    h.settle_at(1).await;
    h.idle().await;
    h.advance(5);
    let _pods = h.service.watch(&cluster(), MetricsKind::Pods);
    h.settle_at(2).await;
    assert!(matches!(h.calls()[1], MetricsCall::PodMetrics(..)));
    h.idle().await;
    // Nodes stay on their own 15 s; pods (forbidden) wait five minutes.
    h.advance(10);
    h.settle_at(3).await;
    assert!(matches!(h.calls()[2], MetricsCall::NodeMetrics(..)));
    assert_eq!(
        h.service
            .state(&cluster(), MetricsKind::Pods)
            .unavailable_text()
            .as_deref(),
        Some("metrics unavailable: no permission to read metrics.k8s.io")
    );
}
