//! The thin fallback decoder: payloads the `k8s-metrics` types reject still yield samples.

use oxikube_domain::metrics::{MetricsSubject, MissingReason, Reading};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::MetricsPort;
use serde_json::json;

use super::{ALL_PODS, NODES, cluster, metrics, node_item, node_list, pod_item, pod_list};
use crate::fake_api::FakeApi;

#[tokio::test]
async fn an_unparseable_window_falls_back_to_lenient_decoding() {
    let api = FakeApi::new();
    let mut item = node_item("cp", "250m", "1Gi");
    item["window"] = json!("about fifteen seconds");
    api.reply(NODES, 200, node_list(vec![item]));
    let outcome = metrics(&api).node_metrics(&cluster()).await.unwrap();

    let samples = outcome.samples().expect("samples");
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].cpu, Reading::Value(250_000_000));
    assert_eq!(samples[0].memory_bytes(), Some(1 << 30));
    assert_eq!(
        samples[0].window, None,
        "the unreadable window is dropped, not guessed"
    );
    assert_eq!(
        samples[0].ts,
        "2026-10-03T12:00:00Z".parse::<jiff::Timestamp>().unwrap()
    );
    assert_eq!(
        api.hits(NODES),
        2,
        "one failed decode, then one fallback read"
    );
}

#[tokio::test]
async fn a_container_without_usage_is_a_missing_reading_not_a_failed_list() {
    let api = FakeApi::new();
    let mut broken = pod_item("kube-system", "new-pod", &[("1m", "1Mi")]);
    broken["containers"] = json!([{"name": "c0"}]);
    let fine = pod_item("kube-system", "ok-pod", &[("5m", "5Mi")]);
    api.reply(ALL_PODS, 200, pod_list(vec![broken, fine]));
    let outcome = metrics(&api)
        .pod_metrics(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();

    let samples = outcome.samples().expect("samples");
    assert_eq!(samples.len(), 2);
    let name = |i: usize| match &samples[i].subject {
        MetricsSubject::Pod { pod } => pod.name.to_string(),
        other => panic!("{other:?}"),
    };
    assert_eq!((name(0).as_str(), name(1).as_str()), ("new-pod", "ok-pod"));
    assert_eq!(samples[0].cpu, Reading::Missing(MissingReason::Unavailable));
    assert_eq!(samples[1].cpu, Reading::Value(5_000_000));
    assert_eq!(api.hits(ALL_PODS), 2);
}

#[tokio::test]
async fn a_decodable_payload_uses_the_crate_types_only() {
    let api = FakeApi::new();
    api.reply(NODES, 200, node_list(vec![node_item("cp", "1", "1Ki")]));
    metrics(&api).node_metrics(&cluster()).await.unwrap();
    assert_eq!(api.hits(NODES), 1);
}
