//! The metrics provider in the app, end to end over fakes: the real settings store, session
//! manager, provider service, detector and PromQL port; only the cluster itself is scripted.

use std::sync::Arc;

use futures::executor::block_on;
use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_app::{
    CommandBus, CommandRegistry, DispatchContext, MutationGuard, Outcome, ProviderStatus,
};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{
    MetricKind, MetricRequest, MetricTarget, MetricsProviderKind, PodSelector, Resource,
};
use oxikube_ports::{ProviderTarget, RangePlan};
use oxikube_settings::SettingsStore;
use oxikube_testkit::{FakeServiceProxyPort, TestPorts};
use serde_json::json;

use super::{SettingsMetricsWriter, start};
use crate::app_state::AppState;

fn cluster() -> ClusterId {
    TestPorts::cluster_id()
}

fn service(namespace: &str, name: &str, port: u16) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": { "name": name, "namespace": namespace },
        "spec": { "ports": [{ "name": "web", "port": port }] }
    }))
    .unwrap()
}

struct Fixture {
    ports: TestPorts,
    state: Arc<AppState>,
    proxy: Arc<FakeServiceProxyPort>,
    _follow: super::MetricsFollow,
}

/// A cluster with `prometheus-operated` in `monitoring`, answering as a Prometheus does.
fn fixture(cx: &mut TestAppContext, with_prometheus: bool) -> Fixture {
    let ports = TestPorts::seeded();
    let connected = ports.connector.ports_for(&cluster());
    if with_prometheus {
        connected
            .resources
            .insert(service("monitoring", "prometheus-operated", 9090));
    }
    connected.service_proxy.respond_with(|service, request| {
        (service.name == "prometheus-operated" && request.path.starts_with("/api/v1/query"))
            .then(|| Ok(FakeServiceProxyPort::prometheus_ok()))
    });
    let proxy = connected.service_proxy;
    let (state, follow) = cx.update(|cx| {
        let state = AppState::test_with(cx, &ports);
        let services = state.services();
        let follow = start(&services.metrics, &services.sessions, cx);
        (state, follow)
    });
    Fixture {
        ports,
        state,
        proxy,
        _follow: follow,
    }
}

impl Fixture {
    fn status(&self) -> Arc<ProviderStatus> {
        self.state.services().metrics.provider.status(&cluster())
    }

    fn connect(&self, cx: &mut TestAppContext) {
        let sessions = self.state.services().sessions.clone();
        block_on(sessions.connect(&cluster())).expect("connect");
        cx.run_until_parked();
    }

    fn edit_settings(&self, cx: &mut TestAppContext, text: &str) {
        let text = text.to_owned();
        cx.update(|cx| {
            cx.update_global::<SettingsStore, _>(|store, _| {
                store.set_user_settings(&text).unwrap();
            });
        });
        cx.run_until_parked();
    }

    fn promql_available(&self) -> bool {
        block_on(
            self.state
                .services()
                .metrics
                .promql
                .is_available(&cluster()),
        )
        .unwrap()
    }
}

#[gpui::test]
fn connecting_a_cluster_with_kube_prometheus_finds_it_and_points_the_query_port_at_it(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx, true);
    assert!(
        !f.promql_available(),
        "nothing is found before the cluster connects"
    );
    f.connect(cx);

    let status = f.status();
    let provider = status.provider().expect("a provider was found");
    assert_eq!(provider.kind, MetricsProviderKind::KubePrometheus);
    let ProviderTarget::Service(service) = &provider.target else {
        panic!("{provider:?}");
    };
    assert_eq!(service.short(), "monitoring/prometheus-operated:web");
    assert!(
        status
            .probes
            .iter()
            .any(|p| p.label() == "lens-metrics/prometheus"),
        "the services looked at are listed"
    );

    // The PromQL port reaches it through the connection's service proxy.
    assert!(f.promql_available());
    let series = block_on(
        f.state
            .services()
            .metrics
            .promql
            .query_instant(&cluster(), "up", None),
    )
    .unwrap();
    assert!(series.is_empty());
    let last = f.proxy.recorded_calls().pop().unwrap();
    assert_eq!(last.service.short(), "monitoring/prometheus-operated:web");
    assert_eq!(last.path, "/api/v1/query");
}

#[gpui::test]
fn the_catalogue_builds_the_query_for_the_provider_found_and_the_port_sends_it(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx, true);
    f.connect(cx);
    let services = f.state.services();
    let kind = f.status().provider().expect("found").kind;

    let request = MetricRequest::new(
        MetricTarget::Pod {
            namespace: "shop".into(),
            pods: PodSelector::One("web-0".into()),
        },
        MetricKind::MemoryUsage,
    );
    let plan = RangePlan::ending_at(
        jiff::Timestamp::from_second(1_800_000_000).unwrap(),
        std::time::Duration::from_secs(3600),
    );
    let query = services
        .metrics
        .catalogue
        .build(kind, &request, plan.rate_window)
        .expect("a pod's memory has a query");
    assert_eq!(query.series_label, Some("pod"));

    block_on(
        services
            .metrics
            .promql
            .query_range(&cluster(), &query.promql, plan.range),
    )
    .unwrap();
    let sent = f.proxy.recorded_calls().pop().unwrap();
    assert_eq!(sent.path, "/api/v1/query_range");
    let sent_query = sent
        .params
        .iter()
        .find(|(name, _)| name == "query")
        .map(|(_, value)| value.as_str());
    assert_eq!(
        sent_query,
        Some(query.promql.as_str()),
        "the catalogue's query reached the service proxy unchanged"
    );
    assert!(
        query.promql.contains("container_memory_working_set_bytes"),
        "{}",
        query.promql
    );
}

#[gpui::test]
fn a_cluster_without_a_stack_says_metrics_unavailable_and_the_port_has_no_endpoint(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx, false);
    f.connect(cx);
    let status = f.status();
    assert!(status.provider().is_none());
    let text = status
        .unavailable_text()
        .expect("absence is a visible state");
    assert!(text.starts_with("metrics unavailable: "), "{text}");
    assert!(!f.promql_available());
}

#[gpui::test]
fn turning_metrics_off_in_settings_stops_the_provider_without_a_reconnect(cx: &mut TestAppContext) {
    let f = fixture(cx, true);
    f.connect(cx);
    assert!(f.status().provider().is_some());
    let connects = f.ports.connector.recorded_calls().len();
    let probes = f.proxy.recorded_calls().len();

    f.edit_settings(
        cx,
        &format!(
            r#"{{ "clusters": {{ "{}": {{ "prometheus": {{ "enabled": false }} }} }} }}"#,
            cluster()
        ),
    );
    let status = f.status();
    assert!(status.unavailable_text().unwrap().contains("turned off"));
    assert!(!f.promql_available(), "the endpoint is forgotten");
    assert_eq!(
        f.ports.connector.recorded_calls().len(),
        connects,
        "no reconnect"
    );
    assert_eq!(
        f.proxy.recorded_calls().len(),
        probes,
        "nothing probed while off"
    );

    // And back on: detection runs again by itself.
    f.edit_settings(cx, "{}");
    assert!(f.status().provider().is_some());
    assert!(f.promql_available());
}

#[gpui::test]
fn a_manual_endpoint_in_settings_replaces_detection(cx: &mut TestAppContext) {
    let f = fixture(cx, true);
    f.connect(cx);
    let probes = f.proxy.recorded_calls().len();

    // A Service reached by hand, with the stack named and a path prefix the fake answers.
    f.proxy.respond_with(|service, request| {
        (service.name == "vmsingle-vmks" && request.path == "/select/0/prometheus/api/v1/query")
            .then(|| Ok(FakeServiceProxyPort::prometheus_ok()))
    });
    f.edit_settings(
        cx,
        &format!(
            r#"{{ "clusters": {{ "{}": {{ "prometheus": {{
                "path": "monitoring/vmsingle-vmks:8429", "kind": "victoria_metrics",
                "path_prefix": "/select/0/prometheus" }} }} }} }}"#,
            cluster()
        ),
    );
    let status = f.status();
    let provider = status.provider().expect("the configured endpoint answered");
    assert_eq!(provider.kind, MetricsProviderKind::VictoriaMetrics);
    assert_eq!(
        provider.path_prefix.as_deref(),
        Some("/select/0/prometheus")
    );
    assert_eq!(
        f.proxy.recorded_calls().len(),
        probes + 1,
        "one check of the endpoint, no search"
    );
    assert!(f.promql_available());
}

#[gpui::test]
fn a_manual_endpoint_that_does_not_answer_is_a_visible_failure(cx: &mut TestAppContext) {
    let f = fixture(cx, true);
    f.connect(cx);
    f.edit_settings(
        cx,
        &format!(
            r#"{{ "clusters": {{ "{}": {{ "prometheus": {{ "path": "monitoring/nothing:9090" }} }} }} }}"#,
            cluster()
        ),
    );
    let text = f.status().unavailable_text().unwrap();
    assert!(text.contains("monitoring/nothing:9090"), "{text}");
    assert!(!f.promql_available());
}

#[gpui::test]
async fn the_toggle_command_edits_settings_json_and_the_provider_follows(cx: &mut TestAppContext) {
    let f = fixture(cx, true);
    f.connect(cx);
    let services = f.state.services().clone();
    let writer = cx.update(|cx| Arc::new(SettingsMetricsWriter::new(cx)));
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::metrics_provider", |r| {
            oxikube_app::metrics_provider::register_commands(
                r,
                services.metrics.provider.clone(),
                writer.clone(),
            )
        })
        .unwrap();
    let guard = MutationGuard::new(
        services.sessions.clone(),
        f.ports.state.clone(),
        f.ports.clock.clone(),
    );
    let bus = CommandBus::new(registry, guard);

    let outcome = bus
        .dispatch(
            Command::ClusterToggleMetrics {
                cluster: cluster(),
                enabled: None,
            },
            DispatchContext::new(Initiator::Ui, "test"),
        )
        .await
        .expect("the command ran");
    assert!(matches!(outcome, Outcome::Completed(_)));
    cx.run_until_parked();

    let status = f.status();
    assert!(
        status.unavailable_text().unwrap().contains("turned off"),
        "{status:?}"
    );
    assert!(!f.promql_available());
    let written = cx.update(|cx| {
        cx.global::<SettingsStore>()
            .user_settings_text()
            .unwrap_or_default()
            .to_owned()
    });
    assert!(written.contains("\"enabled\": false"), "{written}");
    assert!(
        written.contains("kind-oxikube"),
        "the new block is named for the context: {written}"
    );
}
