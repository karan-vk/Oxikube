//! The catalogue's queries and recorded-shape responses agree: each fixture is served only for
//! the exact query the catalogue builds, parsed by the real client, and its series carry the
//! label the query promises (`MetricQuery::series_label`).

mod common;

use std::time::Duration;

use common::{endpoint, fixture};
use jiff::Timestamp;
use oxikube_domain::{
    MetricKind, MetricQuery, MetricRequest, MetricTarget, MetricsProviderKind, PodSelector,
};
use oxikube_ports::{PromqlEndpointPort, PromqlSeries, QueryCataloguePort, RangePlan};
use oxikube_prometheus::{PromqlCatalogue, PromqlClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `(fixture case, request, the label values the fixture's series carry)`.
fn cases() -> Vec<(&'static str, MetricRequest, Vec<&'static str>)> {
    vec![
        (
            "pod-cpu",
            MetricRequest::new(
                MetricTarget::Pod {
                    namespace: "shop".into(),
                    pods: PodSelector::Any(vec!["web-7d9f-abcde".into(), "web-7d9f-fghij".into()]),
                },
                MetricKind::CpuUsage,
            ),
            vec!["web-7d9f-abcde", "web-7d9f-fghij"],
        ),
        (
            "container-memory",
            MetricRequest::new(
                MetricTarget::Container {
                    namespace: "shop".into(),
                    pod: "web-0".into(),
                    container: None,
                },
                MetricKind::MemoryUsage,
            ),
            vec!["app", "sidecar"],
        ),
        (
            "cluster-cpu",
            MetricRequest::new(MetricTarget::Cluster, MetricKind::CpuUsage),
            vec![],
        ),
        (
            "ingress-rx",
            MetricRequest::new(
                MetricTarget::Ingress {
                    namespace: "shop".into(),
                    name: "web".into(),
                },
                MetricKind::NetworkRx,
            ),
            vec![],
        ),
    ]
}

/// The path prefix a provider's API is served under.
fn prefix(provider: MetricsProviderKind) -> &'static str {
    match provider {
        MetricsProviderKind::Mimir => "/prometheus",
        _ => "",
    }
}

fn plan() -> RangePlan {
    RangePlan::ending_at(
        Timestamp::from_second(1_791_707_599).unwrap(),
        Duration::from_secs(15 * 60),
    )
}

async fn serve(server: &MockServer, provider: MetricsProviderKind, promql: &str, body: String) {
    Mock::given(method("GET"))
        .and(path(format!("{}/api/v1/query_range", prefix(provider))))
        .and(query_param("query", promql))
        .and(query_param("step", "15"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(body.into_bytes(), "application/json"),
        )
        .mount(server)
        .await;
}

async fn run(
    server: &MockServer,
    provider: MetricsProviderKind,
    query: &MetricQuery,
    plan: &RangePlan,
) -> Result<Vec<PromqlSeries>, oxikube_ports::PromqlError> {
    let endpoint = endpoint(server).with_path_prefix(prefix(provider));
    PromqlClient::new()
        .range_query(&endpoint, &query.promql, plan.range)
        .await
}

#[tokio::test]
async fn every_provider_parses_its_responses_into_the_labels_the_query_promises() {
    let plan = plan();
    for provider in MetricsProviderKind::ALL {
        for (case, request, expected) in cases() {
            let who = format!("{provider} {case}");
            let query = PromqlCatalogue
                .build(provider, &request, plan.rate_window)
                .unwrap();
            let server = MockServer::start().await;
            serve(
                &server,
                provider,
                &query.promql,
                fixture(&format!("queries/{}-{case}", provider.id())),
            )
            .await;

            let series = run(&server, provider, &query, &plan)
                .await
                .unwrap_or_else(|e| panic!("{who}: {e}\n{}", query.promql));

            match query.series_label {
                Some(label) => {
                    let names: Vec<&str> =
                        series.iter().map(|s| s.labels[label].as_str()).collect();
                    assert_eq!(names, expected, "{who}");
                    assert!(series.iter().all(|s| s.labels.len() == 1), "{who}");
                }
                None => {
                    assert_eq!(series.len(), 1, "{who}");
                    assert!(series[0].labels.is_empty(), "{who}: {:?}", series[0].labels);
                }
            }
            for s in &series {
                assert_eq!(s.points.len(), 5, "{who}");
                assert!(s.points.windows(2).all(|w| w[0].ts < w[1].ts), "{who}");
                assert!(
                    s.points
                        .iter()
                        .all(|p| p.value.is_finite() && p.value > 0.0),
                    "{who}"
                );
                assert_eq!(s.points.last().unwrap().ts, plan.range.end, "{who}");
            }
        }
    }
}

#[tokio::test]
async fn a_changed_query_is_not_answered_by_the_fixture() {
    // The fixture is keyed on the exact query text: a different provider's query for the same
    // request (here an ingress, whose namespace label differs) gets no answer.
    let plan = plan();
    let (case, request, _) = cases().pop().unwrap();
    let kube = PromqlCatalogue
        .build(
            MetricsProviderKind::KubePrometheus,
            &request,
            plan.rate_window,
        )
        .unwrap();
    let lens = PromqlCatalogue
        .build(MetricsProviderKind::Lens, &request, plan.rate_window)
        .unwrap();
    assert_ne!(kube.promql, lens.promql);

    let server = MockServer::start().await;
    serve(
        &server,
        MetricsProviderKind::KubePrometheus,
        &kube.promql,
        fixture(&format!("queries/kube_prometheus-{case}")),
    )
    .await;
    assert!(
        run(&server, MetricsProviderKind::KubePrometheus, &kube, &plan)
            .await
            .is_ok()
    );
    let error = run(&server, MetricsProviderKind::Lens, &lens, &plan)
        .await
        .unwrap_err();
    assert!(
        matches!(error, oxikube_ports::PromqlError::BadResponse(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn the_step_and_window_follow_the_range() {
    // The query's rate window and the request's step come from one policy.
    let request = MetricRequest::new(MetricTarget::Namespace("shop".into()), MetricKind::CpuUsage);
    let end = Timestamp::from_second(1_791_707_599).unwrap();
    for (span_secs, step, window) in [
        (900, 15, "2m"),
        (3600, 15, "2m"),
        (21_600, 120, "4m"),
        (86_400, 300, "10m"),
    ] {
        let plan = RangePlan::ending_at(end, Duration::from_secs(span_secs));
        assert_eq!(plan.range.step, Duration::from_secs(step), "{span_secs}s");
        let query = PromqlCatalogue
            .build(
                MetricsProviderKind::KubePrometheus,
                &request,
                plan.rate_window,
            )
            .unwrap();
        assert!(
            query.promql.ends_with(&format!("[{window}]))")),
            "{span_secs}s: {}",
            query.promql
        );
    }
}
