//! Recorded `NodeMetricsList` / `PodMetricsList` payloads decoded by both paths (the
//! `k8s-metrics` types and the thin fallback) and compared: the same fixture must give the same
//! domain samples, whichever decoder ran.

use jiff::{SignedDuration, Timestamp};
use k8s_metrics::v1beta1::{NodeMetrics, PodMetrics};
use kube::core::{DynamicObject, ObjectList};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason, Reading};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::MetricsPort;
use serde::Deserialize;

use super::{ALL_PODS, NODES, cluster, metrics};
use crate::fake_api::FakeApi;
use crate::metrics::convert::{node_sample, pod_sample};
use crate::metrics::raw::{RawNode, RawPod};

const NODE_LIST: &str = include_str!("fixtures/node_metrics_list.json");
const POD_LIST: &str = include_str!("fixtures/pod_metrics_list.json");

fn fetched_at() -> Timestamp {
    "2026-10-04T00:00:00Z".parse().unwrap()
}

fn list<T: for<'de> Deserialize<'de> + Clone>(text: &str) -> Vec<T> {
    serde_json::from_str::<ObjectList<T>>(text).unwrap().items
}

fn nodes_via_crate() -> Vec<MetricsSample> {
    list::<NodeMetrics>(NODE_LIST)
        .into_iter()
        .map(|n| node_sample(RawNode::from(n), fetched_at()))
        .collect()
}

fn nodes_via_fallback() -> Vec<MetricsSample> {
    list::<DynamicObject>(NODE_LIST)
        .into_iter()
        .map(|n| node_sample(RawNode::from_dynamic(n), fetched_at()))
        .collect()
}

fn pods_via_crate() -> Vec<MetricsSample> {
    list::<PodMetrics>(POD_LIST)
        .into_iter()
        .map(|p| pod_sample(&cluster(), RawPod::from(p), fetched_at()))
        .collect()
}

fn pods_via_fallback() -> Vec<MetricsSample> {
    list::<DynamicObject>(POD_LIST)
        .into_iter()
        .map(|p| pod_sample(&cluster(), RawPod::from_dynamic(p), fetched_at()))
        .collect()
}

#[test]
fn both_decoders_give_identical_node_samples() {
    assert_eq!(nodes_via_crate(), nodes_via_fallback());
}

#[test]
fn both_decoders_give_identical_pod_samples() {
    assert_eq!(pods_via_crate(), pods_via_fallback());
}

#[test]
fn node_fixture_values_are_exact() {
    for samples in [nodes_via_crate(), nodes_via_fallback()] {
        assert_eq!(samples.len(), 3);
        // CPU: nanocores `n`, millicores `m` and whole cores; memory: `Ki`, `Gi`, `Mi`.
        assert_eq!(samples[0].cpu, Reading::Value(196_382_978));
        assert_eq!(samples[0].memory, Reading::Value(1_848_836 * 1024));
        assert_eq!(samples[0].window, Some(SignedDuration::from_millis(14_982)));
        assert_eq!(samples[1].cpu, Reading::Value(3_491_000_000));
        assert_eq!(samples[1].memory, Reading::Value(3 << 30));
        assert_eq!(samples[2].cpu, Reading::Value(2_000_000_000));
        assert_eq!(samples[2].memory, Reading::Value(512 << 20));
        assert_eq!(samples[2].window, Some(SignedDuration::from_secs(60)));
        assert_eq!(
            samples[1].ts,
            "2026-10-03T12:00:02Z".parse::<Timestamp>().unwrap()
        );
    }
}

#[test]
fn a_multi_container_pod_keeps_one_entry_per_container() {
    for samples in [pods_via_crate(), pods_via_fallback()] {
        assert_eq!(samples.len(), 3);
        let web = &samples[1];
        let names: Vec<&str> = web.containers.iter().map(|c| &*c.name).collect();
        assert_eq!(names, ["nginx", "sidecar", "exporter"]);
        assert_eq!(web.containers[0].cpu, Reading::Value(250_000_000));
        assert_eq!(web.containers[1].cpu, Reading::Value(303_000));
        assert_eq!(web.containers[1].memory, Reading::Value(1 << 30));
        assert_eq!(web.containers[2].cpu, Reading::Value(1_000_000_000));
        assert_eq!(web.containers[2].memory, Reading::Value(5_242_880));
        // The pod total is the exact sum of its containers.
        assert_eq!(web.cpu, Reading::Value(1_250_303_000));
        assert_eq!(
            web.memory,
            Reading::Value((64 << 20) + (1 << 30) + 5_242_880)
        );
        let MetricsSubject::Pod { pod } = &web.subject else {
            panic!("a pod subject")
        };
        assert_eq!(pod.namespace(), Some("default"));

        // A pod metrics-server has registered but not scraped yet has no containers.
        assert!(samples[2].containers.is_empty());
        assert_eq!(
            samples[2].cpu,
            Reading::Missing(MissingReason::NotYetAvailable)
        );
    }
}

#[tokio::test]
async fn the_port_serves_the_fixtures_end_to_end() {
    let api = FakeApi::new();
    api.reply(NODES, 200, serde_json::from_str(NODE_LIST).unwrap());
    api.reply(ALL_PODS, 200, serde_json::from_str(POD_LIST).unwrap());
    let port = metrics(&api);

    let nodes = port.node_metrics(&cluster()).await.unwrap();
    assert_eq!(nodes.samples().unwrap(), nodes_via_crate());
    let pods = port
        .pod_metrics(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    assert_eq!(pods.samples().unwrap().len(), 3);
}
