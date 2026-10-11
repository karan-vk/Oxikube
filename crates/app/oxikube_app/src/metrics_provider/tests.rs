//! The service against fake sessions, a fake detector and a recording sink. Runs are tasks on
//! the Tokio test runtime; waits are on the service's own change stream, never a sleep.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use oxikube_domain::MetricsProviderKind;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{
    ClusterContext, ClusterPrefs, ClusterPrefsTable, DetectedProvider, DetectionFailure,
    DetectionResult, DetectionState, MetricsMode, Probe, ProbeOutcome, PrometheusAuthKind,
    PrometheusOverride, PromqlError, ProviderTarget, ServicePort, ServiceRef, SourceId,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeProviderDetectorPort,
    ProviderDetectorCall,
};
use parking_lot::Mutex;

use super::{
    EndpointSink, MetricsProviderService, NoSink, ProviderState, ProviderStatus, Resolved,
};
use crate::session::ClusterSessionManager;
use crate::store::Spawner;

fn context(name: &str) -> ClusterContext {
    let context = ContextName::new(name);
    ClusterContext {
        server: Some(format!("https://{name}.example:6443")),
        ..ClusterContext::new(
            ClusterId::new("/home/me/.kube/config", &context),
            context,
            SourceId("kubeconfig".into()),
        )
    }
}

fn id(name: &str) -> ClusterId {
    context(name).cluster
}

fn operated() -> DetectedProvider {
    DetectedProvider {
        kind: MetricsProviderKind::KubePrometheus,
        target: ProviderTarget::Service(ServiceRef::new(
            "monitoring",
            "prometheus-operated",
            ServicePort::Name("web".into()),
        )),
        path_prefix: None,
    }
}

fn found() -> DetectionResult {
    DetectionResult {
        state: DetectionState::Found(operated()),
        probes: vec![Probe {
            namespace: "monitoring".into(),
            service: "prometheus-operated".into(),
            kind: MetricsProviderKind::KubePrometheus,
            outcome: ProbeOutcome::Answered,
        }],
        finished_at: None,
    }
}

#[derive(Default)]
struct RecordingSink {
    published: Mutex<Vec<(ClusterId, Option<DetectedProvider>)>>,
    endpoints: Mutex<Vec<(ClusterId, String)>>,
}

impl RecordingSink {
    fn last(&self, cluster: &ClusterId) -> Option<Option<DetectedProvider>> {
        self.published
            .lock()
            .iter()
            .rev()
            .find(|(c, _)| c == cluster)
            .map(|(_, p)| p.clone())
    }
}

impl EndpointSink for RecordingSink {
    fn publish(&self, cluster: &ClusterId, resolved: Option<&Resolved>) {
        self.published
            .lock()
            .push((cluster.clone(), resolved.map(|r| r.provider.clone())));
        if let Some(resolved) = resolved {
            self.endpoints
                .lock()
                .push((cluster.clone(), resolved.endpoint.location()));
        }
    }
}

struct Harness {
    sessions: ClusterSessionManager,
    detector: Arc<FakeProviderDetectorPort>,
    sink: Arc<RecordingSink>,
    service: MetricsProviderService,
    changes: BoxStream<'static, ()>,
}

fn tokio_spawner() -> Arc<dyn Spawner> {
    Arc::new(|task: futures::future::BoxFuture<'static, ()>| {
        tokio::spawn(task);
    })
}

impl Harness {
    fn new() -> Self {
        Self::build(ClusterPrefsTable::default(), true)
    }

    fn with_prefs(prefs: PrometheusOverride) -> Self {
        let table = ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            id("a"),
            ClusterPrefs {
                prometheus: Some(prefs),
                ..ClusterPrefs::default()
            },
        );
        Self::build(table, true)
    }

    fn build(table: ClusterPrefsTable, start: bool) -> Self {
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source =
            Arc::new(FakeClusterSourcePort::new().with_contexts([context("a"), context("b")]));
        let sessions =
            ClusterSessionManager::new(connector, source, Arc::new(FakeClockPort::default()));
        sessions.set_prefs_table(table);
        let detector = Arc::new(FakeProviderDetectorPort::new());
        let sink = Arc::new(RecordingSink::default());
        let service = MetricsProviderService::new(sessions.clone(), detector.clone(), sink.clone());
        let changes = service.changes(&id("a"));
        if start {
            service.start(tokio_spawner());
        }
        Self {
            sessions,
            detector,
            sink,
            service,
            changes,
        }
    }

    async fn connect(&self, name: &str) {
        self.sessions.connect(&id(name)).await.expect("connect");
    }

    /// Waits for the next change of cluster `a`.
    async fn next_change(&mut self) {
        tokio::time::timeout(Duration::from_secs(5), self.changes.next())
            .await
            .expect("a status change")
            .expect("the stream is open");
    }

    /// Waits until `a`'s status satisfies `done`.
    async fn until(&mut self, done: impl Fn(&ProviderStatus) -> bool) -> Arc<ProviderStatus> {
        loop {
            let status = self.service.status(&id("a"));
            if done(&status) {
                return status;
            }
            self.next_change().await;
        }
    }

    /// Waits until the detector has been asked `n` times: the runs are tasks, and a status of
    /// `Detecting` only says one was started.
    async fn detects_reach(&self, n: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.detects() < n {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the detector was asked");
    }

    fn detects(&self) -> usize {
        self.detector
            .recorded_calls()
            .iter()
            .filter(|c| matches!(c, ProviderDetectorCall::Detect))
            .count()
    }
}

fn manual() -> PrometheusOverride {
    PrometheusOverride {
        mode: MetricsMode::Manual,
        ..PrometheusOverride::default()
    }
}

#[tokio::test]
async fn a_connecting_cluster_is_detected_once_and_the_provider_reaches_the_sink() {
    let mut h = Harness::new();
    h.detector.script().detect.push_ok(found());
    assert!(matches!(
        h.service.status(&id("a")).state,
        ProviderState::WaitingForCluster
    ));

    h.connect("a").await;
    let status = h.until(|s| s.provider().is_some()).await;

    assert_eq!(
        status.provider().unwrap().kind,
        MetricsProviderKind::KubePrometheus
    );
    assert_eq!(status.probes.len(), 1);
    assert_eq!(h.detects(), 1);
    assert_eq!(h.sink.last(&id("a")), Some(Some(operated())));
    assert_eq!(h.sink.last(&id("b")), None, "other clusters are untouched");
    assert_eq!(
        h.sink.endpoints.lock().as_slice(),
        [(id("a"), "monitoring/prometheus-operated:web".to_owned())],
        "queries are pointed at the service"
    );
}

#[tokio::test]
async fn nothing_found_is_a_visible_state_with_a_reason() {
    let mut h = Harness::new();
    h.detector
        .script()
        .detect
        .push_ok(DetectionResult::of(DetectionState::NotFound));
    h.connect("a").await;
    let status = h
        .until(|s| matches!(s.state, ProviderState::Detection(DetectionState::NotFound)))
        .await;
    let text = status.unavailable_text().unwrap();
    assert!(text.starts_with("metrics unavailable: "), "{text}");
    assert_eq!(h.sink.last(&id("a")), Some(None));
}

#[tokio::test]
async fn a_forbidden_list_surfaces_as_a_failure_not_an_absence() {
    let mut h = Harness::new();
    h.detector
        .script()
        .detect
        .push_ok(DetectionResult::of(DetectionState::Failed(
            DetectionFailure::Forbidden("cannot list services in monitoring".into()),
        )));
    h.connect("a").await;
    let status = h
        .until(|s| matches!(s.state, ProviderState::Detection(DetectionState::Failed(_))))
        .await;
    assert!(
        status
            .unavailable_text()
            .unwrap()
            .contains("configure the endpoint by hand")
    );
}

#[tokio::test]
async fn the_status_is_cached_not_re_probed_on_every_read_or_unrelated_change() {
    let mut h = Harness::new();
    h.detector.script().detect.push_ok(found());
    h.connect("a").await;
    h.until(|s| s.provider().is_some()).await;

    for _ in 0..50 {
        let _ = h.service.status(&id("a"));
    }
    // A settings change that leaves this cluster's prometheus settings alone.
    h.sessions
        .set_prefs_table(ClusterPrefsTable::new(ClusterPrefs {
            read_only: true,
            ..ClusterPrefs::default()
        }));
    h.service.settings_changed();
    assert_eq!(h.detects(), 1);
}

#[tokio::test]
async fn a_reconnect_detects_again_and_a_disconnect_forgets_the_endpoint() {
    let mut h = Harness::new();
    h.detector.script().detect.push_ok(found()).push_ok(found());
    h.connect("a").await;
    h.until(|s| s.provider().is_some()).await;

    h.sessions.disconnect(&id("a")).expect("disconnect");
    let status = h
        .until(|s| matches!(s.state, ProviderState::WaitingForCluster))
        .await;
    assert!(status.unavailable_text().unwrap().contains("not connected"));
    assert_eq!(h.sink.last(&id("a")), Some(None));

    h.connect("a").await;
    h.until(|s| s.provider().is_some()).await;
    assert_eq!(h.detects(), 2);
    assert_eq!(h.sink.last(&id("a")), Some(Some(operated())));
}

#[tokio::test]
async fn running_detection_again_replaces_a_run_in_progress_and_ignores_its_result() {
    let mut h = Harness::new();
    let slow = h.detector.hold_next_detect();
    h.connect("a").await;
    h.until(|s| s.is_detecting()).await;
    h.detects_reach(1).await;

    // The first run is stuck. Ask again: a second run takes over and finishes.
    h.detector.script().detect.push_ok(found());
    h.service.redetect(&id("a")).unwrap();
    h.until(|s| s.provider().is_some()).await;
    assert!(slow.is_canceled(), "the replaced run was aborted");
    assert_eq!(h.detects(), 2);
    assert!(h.service.status(&id("a")).provider().is_some());
}

#[tokio::test]
async fn redetect_checks_the_cluster_and_the_start() {
    let h = Harness::build(ClusterPrefsTable::default(), false);
    assert_eq!(
        h.service.redetect(&id("a")).unwrap_err().kind(),
        oxikube_domain::ErrorKind::NotFound,
        "an unopened cluster"
    );
    h.sessions.open_configured(&context("a"));
    assert_eq!(
        h.service.redetect(&id("a")).unwrap_err().kind(),
        oxikube_domain::ErrorKind::Unsupported,
        "before the service started"
    );
}

#[tokio::test]
async fn a_cluster_with_metrics_disabled_is_never_probed_and_says_why() {
    let mut h = Harness::with_prefs(PrometheusOverride {
        enabled: false,
        ..PrometheusOverride::default()
    });
    h.connect("a").await;
    let status = h
        .until(|s| matches!(s.state, ProviderState::Disabled))
        .await;
    assert!(status.unavailable_text().unwrap().contains("turned off"));
    assert_eq!(h.detector.recorded_calls(), vec![]);

    // Switching it on in the settings starts detection without a reconnect.
    h.detector.script().detect.push_ok(found());
    h.sessions.set_prefs_table(ClusterPrefsTable::default());
    h.service.settings_changed();
    h.until(|s| s.provider().is_some()).await;
    assert_eq!(h.detects(), 1);

    // And off again forgets the endpoint.
    h.sessions.set_prefs_table(
        ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            id("a"),
            ClusterPrefs {
                prometheus: Some(PrometheusOverride {
                    mode: MetricsMode::Off,
                    ..PrometheusOverride::default()
                }),
                ..ClusterPrefs::default()
            },
        ),
    );
    h.service.settings_changed();
    h.until(|s| matches!(s.state, ProviderState::Disabled))
        .await;
    assert_eq!(h.sink.last(&id("a")), Some(None));
}

#[tokio::test]
async fn a_manual_url_is_checked_and_reported_with_its_kind_and_prefix() {
    let mut h = Harness::with_prefs(PrometheusOverride {
        url: Some("https://vm.example.com".into()),
        path_prefix: Some("/select/0/prometheus".into()),
        kind: Some(MetricsProviderKind::VictoriaMetrics),
        ..manual()
    });
    h.connect("a").await;
    let status = h.until(|s| s.provider().is_some()).await;
    let provider = status.provider().unwrap();
    assert_eq!(provider.kind, MetricsProviderKind::VictoriaMetrics);
    assert_eq!(
        provider.target,
        ProviderTarget::Url("https://vm.example.com".into())
    );
    assert_eq!(
        provider.path_prefix.as_deref(),
        Some("/select/0/prometheus")
    );
    assert_eq!(
        h.detects(),
        0,
        "a manual endpoint is checked, not searched for"
    );
    assert_eq!(
        h.detector.recorded_calls(),
        vec![ProviderDetectorCall::Check("https://vm.example.com".into())]
    );
    assert!(h.sink.last(&id("a")).unwrap().is_some());
}

#[tokio::test]
async fn a_manual_service_path_goes_through_the_proxy_check() {
    let mut h = Harness::with_prefs(PrometheusOverride {
        path: Some("monitoring/vmsingle-vmks:8429".into()),
        ..manual()
    });
    h.connect("a").await;
    let status = h.until(|s| s.provider().is_some()).await;
    assert_eq!(
        status.provider().unwrap().target.to_string(),
        "monitoring/vmsingle-vmks:8429"
    );
}

#[tokio::test]
async fn a_manual_endpoint_that_does_not_answer_is_a_failure_with_the_reason() {
    let mut h = Harness::with_prefs(PrometheusOverride {
        url: Some("https://vm.example.com".into()),
        ..manual()
    });
    h.detector
        .script()
        .push_check(Err(PromqlError::Unauthorized("401 Unauthorized".into())));
    h.connect("a").await;
    let status = h
        .until(|s| matches!(s.state, ProviderState::Detection(DetectionState::Failed(_))))
        .await;
    let text = status.unavailable_text().unwrap();
    assert!(
        text.contains("https://vm.example.com") && text.contains("rejected the credentials"),
        "{text}"
    );
    assert_eq!(h.sink.last(&id("a")), Some(None));
}

#[tokio::test]
async fn a_manual_block_that_cannot_form_an_endpoint_says_what_is_missing() {
    let mut h = Harness::with_prefs(PrometheusOverride {
        url: Some("https://prom.example.com".into()),
        auth: PrometheusAuthKind::Bearer,
        ..manual()
    });
    h.connect("a").await;
    let status = h
        .until(|s| matches!(s.state, ProviderState::Detection(DetectionState::Failed(_))))
        .await;
    assert!(status.unavailable_text().unwrap().contains("auth_secret"));
    assert!(h.detector.recorded_calls().is_empty(), "nothing is sent");
}

#[tokio::test]
async fn changing_the_settings_of_a_connected_cluster_runs_the_new_mode() {
    let mut h = Harness::new();
    h.detector.script().detect.push_ok(found());
    h.connect("a").await;
    h.until(|s| s.provider().is_some()).await;

    let prefs = PrometheusOverride {
        url: Some("https://prom.example.com".into()),
        ..manual()
    };
    h.sessions.set_prefs_table(
        ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            id("a"),
            ClusterPrefs {
                prometheus: Some(prefs),
                ..ClusterPrefs::default()
            },
        ),
    );
    h.service.settings_changed();
    h.until(|s| {
        s.provider()
            .is_some_and(|p| matches!(p.target, ProviderTarget::Url(_)))
    })
    .await;
    assert_eq!(h.detects(), 1);
}

#[tokio::test]
async fn closing_a_cluster_forgets_its_status_and_endpoint() {
    let mut h = Harness::new();
    h.detector.script().detect.push_ok(found());
    h.connect("a").await;
    h.until(|s| s.provider().is_some()).await;
    h.sessions.close(&id("a"));
    // The follower sees the disconnect and the close; both end with the endpoint forgotten.
    while h.sink.last(&id("a")) != Some(None) {
        h.next_change().await;
    }
}

#[tokio::test]
async fn dropping_the_service_aborts_a_run_in_progress() {
    let mut h = Harness::new();
    let mut gate = h.detector.hold_next_detect();
    h.connect("a").await;
    h.until(|s| s.is_detecting()).await;
    h.detects_reach(1).await;
    let Harness { service, .. } = h;
    drop(service);
    tokio::time::timeout(Duration::from_secs(5), gate.cancellation())
        .await
        .expect("the run is aborted once the service is gone");
}

#[tokio::test]
async fn starting_twice_keeps_the_first_spawner_and_does_not_double_the_runs() {
    let mut h = Harness::new();
    h.service.start(tokio_spawner());
    h.detector.script().detect.push_ok(found());
    h.connect("a").await;
    h.until(|s| s.provider().is_some()).await;
    assert_eq!(h.detects(), 1);
}

#[tokio::test]
async fn clusters_already_connected_when_the_service_starts_are_picked_up() {
    let mut h = Harness::build(ClusterPrefsTable::default(), false);
    h.detector.script().detect.push_ok(found());
    h.connect("a").await;
    h.service.start(tokio_spawner());
    h.until(|s| s.provider().is_some()).await;
    assert_eq!(h.detects(), 1);
}

#[test]
fn the_no_op_sink_accepts_anything() {
    NoSink.publish(&id("a"), None);
}

#[tokio::test]
async fn a_setting_that_changes_no_probe_only_updates_what_the_page_shows() {
    let mut h = Harness::new();
    h.detector.script().detect.push_ok(found());
    h.connect("a").await;
    h.until(|s| s.provider().is_some()).await;

    // `kind` only names the stack for manual endpoints: auto detection stays as it was.
    h.sessions.set_prefs_table(
        ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            id("a"),
            ClusterPrefs {
                prometheus: Some(PrometheusOverride {
                    kind: Some(MetricsProviderKind::Mimir),
                    ..PrometheusOverride::default()
                }),
                ..ClusterPrefs::default()
            },
        ),
    );
    h.service.settings_changed();
    let status = h
        .until(|s| s.settings.kind == Some(MetricsProviderKind::Mimir))
        .await;
    assert!(status.provider().is_some(), "the found provider stays");
    assert_eq!(h.detects(), 1);
}

#[tokio::test]
async fn attaching_alone_spawns_nothing_and_following_is_the_hosts_to_run() {
    let mut h = Harness::build(ClusterPrefsTable::default(), false);
    h.service.attach(tokio_spawner());
    h.detector.script().detect.push_ok(found());

    // Nobody follows the sessions yet: a connect changes nothing.
    h.connect("a").await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(h.detects(), 0);
    assert!(matches!(
        h.service.status(&id("a")).state,
        ProviderState::WaitingForCluster
    ));

    // The host runs `follow` where it likes; from then on updates are seen.
    let following = tokio::spawn(h.service.follow());
    h.sessions.disconnect(&id("a")).expect("disconnect");
    h.connect("a").await;
    h.until(|s| s.provider().is_some()).await;
    assert_eq!(h.detects(), 1);
    following.abort();
}
