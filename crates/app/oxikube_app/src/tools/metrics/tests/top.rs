//! `k8s.top_pods` and `k8s.top_nodes`: sorted, bounded, and honest about absence.

use oxikube_domain::OxiError;
use oxikube_domain::metrics::MissingReason;
use oxikube_domain::session::NamespaceSelection;
use oxikube_testkit::MetricsCall;
use serde_json::json;

use super::*;
use crate::tools::metrics::{TOP_NODES, TOP_PODS};

fn pods() -> Vec<MetricsSample> {
    vec![
        pod_sample("shop", "api-1", at(30), 250, 300),
        pod_sample("shop", "web-0", at(30), 900, 100),
        pod_sample("kube-system", "dns", at(31), 40, 20),
        pod_sample("shop", "db-0", at(29), 120, 900),
        pod_sample("jobs", "batch", at(30), 900, 50),
    ]
}

fn names(data: &Value) -> Vec<String> {
    data["pods"]
        .as_array()
        .expect("pods")
        .iter()
        .map(|p| {
            format!(
                "{}/{}",
                p["namespace"].as_str().unwrap(),
                p["name"].as_str().unwrap()
            )
        })
        .collect()
}

#[tokio::test]
async fn top_pods_ranks_by_cpu_by_default_and_breaks_ties_by_name() {
    let h = Harness::new();
    h.connect().await;
    h.metrics().script().pod_metrics.push_ok(available(pods()));
    let out = h.call(TOP_PODS, json!({})).await.unwrap();
    let data = payload(&out);
    assert_eq!(data["status"], "ok");
    assert_eq!(data["sort"], "cpu");
    assert_eq!(data["total_pods"], 5);
    assert_eq!(
        names(data),
        [
            "jobs/batch",
            "shop/web-0",
            "shop/api-1",
            "shop/db-0",
            "kube-system/dns"
        ]
    );
    let first = &data["pods"][0];
    assert_eq!(first["cpu_millicores"], 900);
    assert_eq!(first["cpu"], "900m");
    assert_eq!(first["memory_bytes"], 50 * MI);
    assert_eq!(first["memory"], "50Mi");
    assert_eq!(
        data["sampled_at"],
        at(31).to_string(),
        "the newest sample's time"
    );
    let text = text(&out);
    assert!(
        text.starts_with("top 5 of 5 pods by cpu in all namespaces on "),
        "{text}"
    );
    assert!(text.contains("jobs\tbatch\t900m\t50Mi"), "{text}");
}

#[tokio::test]
async fn top_pods_sorts_by_memory_and_honours_the_limit() {
    let h = Harness::new();
    h.connect().await;
    h.metrics().script().pod_metrics.push_ok(available(pods()));
    let out = h
        .call(TOP_PODS, json!({"sort": "memory", "limit": 2}))
        .await
        .unwrap();
    let data = payload(&out);
    assert_eq!(names(data), ["shop/db-0", "shop/api-1"]);
    assert_eq!(data["returned"], 2);
    assert_eq!(data["total_pods"], 5, "the total says what was left out");
}

#[tokio::test]
async fn top_pods_asks_for_one_namespace_only_when_told() {
    let h = Harness::new();
    h.connect().await;
    let metrics = h.metrics();
    metrics.script().pod_metrics.push_ok(available(vec![]));
    metrics.script().pod_metrics.push_ok(available(vec![]));
    let out = h
        .call(TOP_PODS, json!({"namespace": "shop"}))
        .await
        .unwrap();
    assert_eq!(payload(&out)["namespace"], "shop");
    h.call(TOP_PODS, json!({})).await.unwrap();
    assert_eq!(
        metrics.recorded_calls(),
        [
            MetricsCall::PodMetrics(cluster(), NamespaceSelection::from_names(["shop"])),
            MetricsCall::PodMetrics(cluster(), NamespaceSelection::All),
        ]
    );
}

#[tokio::test]
async fn a_pod_without_a_reading_sorts_last_and_says_why() {
    let h = Harness::new();
    h.connect().await;
    let mut gone = pod_sample("shop", "new-0", at(30), 0, 0);
    gone.cpu = Reading::Missing(MissingReason::NotYetAvailable);
    gone.memory = Reading::Missing(MissingReason::NotYetAvailable);
    h.metrics().script().pod_metrics.push_ok(available(vec![
        gone,
        pod_sample("shop", "web-0", at(30), 10, 10),
    ]));
    let out = h.call(TOP_PODS, json!({})).await.unwrap();
    let data = payload(&out);
    assert_eq!(names(data), ["shop/web-0", "shop/new-0"]);
    assert_eq!(data["pods"][1]["cpu_millicores"], Value::Null, "not a zero");
    assert!(data["pods"][1]["cpu"].as_str().unwrap().starts_with("n/a"));
}

#[tokio::test]
async fn no_metrics_server_is_an_unavailable_result_with_the_reason() {
    let h = Harness::new();
    h.connect().await;
    for tool in [TOP_PODS, TOP_NODES] {
        h.metrics()
            .script()
            .pod_metrics
            .push_ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled));
        h.metrics()
            .script()
            .node_metrics
            .push_ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled));
        let out = h.call(tool, json!({})).await.unwrap();
        assert!(!out.is_error, "absence is a result, not a failure");
        let data = payload(&out);
        assert_eq!(data["status"], "unavailable");
        assert!(
            data["reason"]
                .as_str()
                .unwrap()
                .contains("metrics-server is not installed")
        );
        assert!(
            text(&out).starts_with("metrics unavailable: "),
            "{}",
            text(&out)
        );
        assert!(
            data.get("pods").is_none() && data.get("nodes").is_none(),
            "no zeroes"
        );
    }
}

#[tokio::test]
async fn a_forbidden_or_failed_read_is_unavailable_too() {
    let h = Harness::new();
    h.connect().await;
    h.metrics()
        .script()
        .pod_metrics
        .push_ok(MetricsOutcome::Unavailable(MissingReason::Forbidden));
    let out = h.call(TOP_PODS, json!({})).await.unwrap();
    assert!(
        payload(&out)["reason"]
            .as_str()
            .unwrap()
            .contains("no permission")
    );

    h.metrics()
        .script()
        .pod_metrics
        .push_err(OxiError::network("connection refused by 10.0.0.1"));
    let out = h.call(TOP_PODS, json!({})).await.unwrap();
    assert_eq!(payload(&out)["status"], "unavailable");
    assert!(text(&out).contains("connection refused"), "{}", text(&out));
}

#[tokio::test]
async fn top_pods_names_the_cluster_when_several_are_connected_or_none() {
    let h = Harness::new();
    let none = h.call(TOP_PODS, json!({})).await.unwrap();
    assert!(
        none.is_error,
        "no cluster is connected: a tool error the model sees"
    );
    assert!(
        text(&none).contains("no cluster is connected"),
        "{}",
        text(&none)
    );
}

#[tokio::test]
async fn top_nodes_ranks_and_gives_the_percent_of_allocatable() {
    let h = Harness::new();
    h.connect().await;
    h.objects().insert(node_object("n1", "4", "8Gi"));
    h.objects().insert(node_object("n2", "2", "4Gi"));
    h.metrics().script().node_metrics.push_ok(available(vec![
        node_sample("n1", at(30), 1000, 2048),
        node_sample("n2", at(30), 1500, 1024),
        node_sample("n3", at(30), 100, 100),
    ]));
    let out = h.call(TOP_NODES, json!({})).await.unwrap();
    let data = payload(&out);
    assert_eq!(data["total_nodes"], 3);
    let nodes = data["nodes"].as_array().unwrap();
    assert_eq!(nodes[0]["name"], "n2");
    assert_eq!(nodes[0]["cpu_percent_of_allocatable"], 75.0);
    assert_eq!(nodes[0]["memory_percent_of_allocatable"], 25.0);
    assert_eq!(nodes[1]["name"], "n1");
    assert_eq!(nodes[1]["cpu_percent_of_allocatable"], 25.0);
    assert_eq!(nodes[2]["name"], "n3");
    assert_eq!(
        nodes[2]["cpu_percent_of_allocatable"],
        Value::Null,
        "a node whose allocatable is unknown has no percent, not zero"
    );

    h.metrics().script().node_metrics.push_ok(available(vec![
        node_sample("n1", at(30), 1000, 2048),
        node_sample("n2", at(30), 1500, 1024),
    ]));
    let out = h
        .call(TOP_NODES, json!({"sort": "memory", "limit": 1}))
        .await
        .unwrap();
    assert_eq!(payload(&out)["nodes"][0]["name"], "n1");
    assert_eq!(payload(&out)["returned"], 1);
}
