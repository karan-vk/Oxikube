//! The port contract against a scripted API server: samples, absence, failures, request count.

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::metrics::{MetricsSubject, MissingReason, Reading};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{MetricsOutcome, MetricsPort};

use super::{ALL_PODS, NODES, cluster, metrics, node_item, node_list, ns_pods, pod_item, pod_list};
use crate::fake_api::{FakeApi, status_body};

fn available(outcome: MetricsOutcome) -> Vec<oxikube_domain::metrics::MetricsSample> {
    match outcome {
        MetricsOutcome::Available(samples) => samples,
        other => panic!("expected samples, got {other:?}"),
    }
}

#[tokio::test]
async fn node_metrics_lists_every_node_in_one_request() {
    let api = FakeApi::new();
    api.reply(
        NODES,
        200,
        node_list(vec![
            node_item("cp", "196382978n", "1848836Ki"),
            node_item("worker", "3491m", "3Gi"),
        ]),
    );
    let samples = available(metrics(&api).node_metrics(&cluster()).await.unwrap());

    assert_eq!(samples.len(), 2);
    assert_eq!(
        samples[0].subject,
        MetricsSubject::Node { node: "cp".into() }
    );
    assert_eq!(samples[0].cpu, Reading::Value(196_382_978));
    assert_eq!(samples[0].memory, Reading::Value(1_893_208_064));
    assert_eq!(samples[1].cpu_millicores(), Some(3491));
    assert_eq!(samples[1].memory_bytes(), Some(3 << 30));
    assert_eq!(
        samples[0].ts,
        "2026-10-03T12:00:00Z".parse::<jiff::Timestamp>().unwrap()
    );
    assert_eq!(
        samples[0].window,
        Some(jiff::SignedDuration::from_millis(14_982))
    );
    assert_eq!(api.requests().len(), 1, "one list, no per-node requests");
}

#[tokio::test]
async fn pod_metrics_for_all_namespaces_is_one_request_for_any_number_of_pods() {
    let api = FakeApi::new();
    let items = (0..50)
        .map(|i| {
            pod_item(
                "kube-system",
                &format!("p{i}"),
                &[("1m", "1Mi"), ("2m", "2Mi")],
            )
        })
        .collect();
    api.reply(ALL_PODS, 200, pod_list(items));
    let samples = available(
        metrics(&api)
            .pod_metrics(&cluster(), &NamespaceSelection::All)
            .await
            .unwrap(),
    );

    assert_eq!(samples.len(), 50);
    assert_eq!(samples[7].cpu, Reading::Value(3_000_000));
    assert_eq!(samples[7].memory_bytes(), Some(3 * 1024 * 1024));
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, ALL_PODS);
}

#[tokio::test]
async fn pod_metrics_for_a_namespace_set_lists_each_namespace_once() {
    let api = FakeApi::new();
    api.reply(
        &ns_pods("a"),
        200,
        pod_list(vec![pod_item("a", "p1", &[("1m", "1Mi")])]),
    );
    api.reply(
        &ns_pods("b"),
        200,
        pod_list(vec![pod_item("b", "p2", &[("2m", "2Mi")])]),
    );
    let selection = NamespaceSelection::from_names(["b", "a"]);
    let samples = available(
        metrics(&api)
            .pod_metrics(&cluster(), &selection)
            .await
            .unwrap(),
    );

    let mut names: Vec<_> = samples
        .iter()
        .map(|s| match &s.subject {
            MetricsSubject::Pod { pod } => format!("{}/{}", pod.namespace().unwrap(), pod.name),
            other => panic!("{other:?}"),
        })
        .collect();
    names.sort();
    assert_eq!(names, ["a/p1", "b/p2"]);
    assert_eq!(api.hits(&ns_pods("a")), 1);
    assert_eq!(api.hits(&ns_pods("b")), 1);
    assert_eq!(api.hits(ALL_PODS), 0);
}

#[tokio::test]
async fn an_empty_list_is_available_with_no_samples() {
    let api = FakeApi::new();
    api.reply(NODES, 200, node_list(Vec::new()));
    let outcome = metrics(&api).node_metrics(&cluster()).await.unwrap();
    assert_eq!(outcome, MetricsOutcome::Available(Vec::new()));
}

#[tokio::test]
async fn a_404_is_not_installed_not_an_error() {
    let api = FakeApi::new();
    let body = status_body(
        404,
        "NotFound",
        "the server could not find the requested resource",
    );
    api.reply(NODES, 404, body.clone());
    api.reply(ALL_PODS, 404, body.clone());
    api.reply(&ns_pods("a"), 404, body);
    let port = metrics(&api);

    let nodes = port.node_metrics(&cluster()).await.unwrap();
    assert_eq!(
        nodes.unavailable_reason(),
        Some(MissingReason::NotInstalled)
    );
    let pods = port
        .pod_metrics(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    assert_eq!(pods.unavailable_reason(), Some(MissingReason::NotInstalled));
    let set = NamespaceSelection::from_names(["a"]);
    let pods = port.pod_metrics(&cluster(), &set).await.unwrap();
    assert_eq!(pods.unavailable_reason(), Some(MissingReason::NotInstalled));
}

#[tokio::test]
async fn a_503_is_an_unavailable_apiservice_not_an_error() {
    let api = FakeApi::new();
    api.reply(
        NODES,
        503,
        status_body(
            503,
            "ServiceUnavailable",
            "the server is currently unable to handle the request",
        ),
    );
    let outcome = metrics(&api).node_metrics(&cluster()).await.unwrap();
    assert_eq!(
        outcome.unavailable_reason(),
        Some(MissingReason::Unavailable)
    );
}

#[tokio::test]
async fn one_namespace_absent_makes_the_whole_outcome_unavailable() {
    let api = FakeApi::new();
    api.reply(
        &ns_pods("a"),
        200,
        pod_list(vec![pod_item("a", "p", &[("1m", "1Mi")])]),
    );
    api.reply(
        &ns_pods("b"),
        503,
        status_body(503, "ServiceUnavailable", "down"),
    );
    let selection = NamespaceSelection::from_names(["a", "b"]);
    let outcome = metrics(&api)
        .pod_metrics(&cluster(), &selection)
        .await
        .unwrap();
    assert_eq!(
        outcome.unavailable_reason(),
        Some(MissingReason::Unavailable)
    );
}

#[tokio::test]
async fn failures_of_the_call_stay_distinct_from_absence() {
    for (code, reason, kind, retryable) in [
        (403, "Forbidden", ErrorKind::Forbidden, false),
        (401, "Unauthorized", ErrorKind::Auth, false),
        (429, "TooManyRequests", ErrorKind::Network, true),
        (504, "Timeout", ErrorKind::Timeout, true),
        (408, "Timeout", ErrorKind::Timeout, true),
    ] {
        let api = FakeApi::new();
        api.reply(NODES, code, status_body(code, reason, "no"));
        let err = metrics(&api)
            .node_metrics(&cluster())
            .await
            .expect_err(&format!("{code} is an error"));
        assert_eq!(err.kind(), kind, "HTTP {code}");
        if kind != ErrorKind::Auth {
            assert_eq!(err.is_retryable(), retryable, "HTTP {code}");
        }
    }
}

#[tokio::test]
async fn another_clusters_id_is_a_validation_error_and_sends_nothing() {
    let api = FakeApi::new();
    let other = ClusterId::new("test", &ContextName::new("prod"));
    let err = metrics(&api).node_metrics(&other).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    let err = metrics(&api)
        .pod_metrics(&other, &NamespaceSelection::All)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(api.requests().is_empty());
}

#[tokio::test]
async fn the_port_is_object_safe() {
    let api = FakeApi::new();
    api.reply(NODES, 200, node_list(Vec::new()));
    let port: Box<dyn MetricsPort> = Box::new(metrics(&api));
    assert!(port.node_metrics(&cluster()).await.is_ok());
}

/// The performance note of the story: 2 000 pods are one request and well under 1 MB, with no
/// per-pod calls. Run with `--nocapture` for the measured size and decode time.
#[tokio::test]
async fn two_thousand_pods_cost_one_request() {
    let api = FakeApi::new();
    let items = (0..2000)
        .map(|i| {
            pod_item(
                "default",
                &format!("workload-{i:05}-7d9f8b6c5-x2x9q"),
                &[("1234567n", "22272Ki"), ("2m", "5Mi")],
            )
        })
        .collect();
    let body = pod_list(items);
    let bytes = body.to_string().len();
    api.reply(ALL_PODS, 200, body);

    let started = std::time::Instant::now();
    let outcome = metrics(&api)
        .pod_metrics(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    let elapsed = started.elapsed();

    assert_eq!(available(outcome).len(), 2000);
    assert_eq!(api.requests().len(), 1);
    assert!(
        (400_000..1_200_000).contains(&bytes),
        "payload of {bytes} bytes drifted from the 'well under 1 MB' the docs quote"
    );
    #[allow(clippy::print_stdout)]
    {
        println!("2000 pods: 1 request, {bytes} bytes, fetched and converted in {elapsed:?}");
    }
}
