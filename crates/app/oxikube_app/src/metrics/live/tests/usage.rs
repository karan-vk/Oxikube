//! The per-cluster cache: a poll lands as one batch, objects missing from it are evicted,
//! absence clears and is a visible state, a failed call keeps the last samples.

use oxikube_domain::OxiError;
use oxikube_domain::metrics::MissingReason;
use oxikube_ports::MetricsOutcome;

use super::{MI, MILLI, at, node_sample, nodes, pod_sample, t0};
use crate::metrics::live::{ClusterUsage, MetricsKind, MetricsState};
use crate::nodes::{NodeUsage, NodeUsageSource};

#[test]
fn a_node_poll_keeps_a_ring_per_node_and_evicts_nodes_that_left() {
    let usage = ClusterUsage::new();
    assert_eq!(usage.state(MetricsKind::Nodes), MetricsState::Idle);
    let epoch = usage.epoch();
    usage.apply(
        MetricsKind::Nodes,
        at(1),
        &Ok(nodes(vec![
            node_sample("a", t0(), 250, 512),
            node_sample("b", t0(), 100, 256),
        ])),
    );
    assert!(usage.epoch() > epoch, "a poll moves the epoch");
    assert_eq!(
        usage.state(MetricsKind::Nodes),
        MetricsState::Available { at: at(1) }
    );
    assert_eq!(
        usage.node("a").and_then(|p| p.cpu_nanocores),
        Some(250 * MILLI)
    );
    assert_eq!(
        usage.usage("b"),
        Some(NodeUsage {
            cpu_millis: 100,
            memory_bytes: (256 * MI) as i64
        })
    );

    // `b` is gone (deleted): the next poll does not list it.
    usage.apply(
        MetricsKind::Nodes,
        at(16),
        &Ok(nodes(vec![node_sample("a", at(15), 300, 600)])),
    );
    assert_eq!(usage.node("b"), None, "evicted");
    assert_eq!(usage.node_history("a").len(), 2);
    assert_eq!(usage.counts(), (1, 0));
}

#[test]
fn a_pod_poll_keeps_containers_and_drops_pods_that_left() {
    let usage = ClusterUsage::new();
    usage.apply(
        MetricsKind::Pods,
        t0(),
        &Ok(MetricsOutcome::Available(vec![
            pod_sample("shop", "web-0", t0(), 50, 64),
            pod_sample("shop", "web-1", t0(), 70, 80),
            pod_sample("kube-system", "dns", t0(), 5, 20),
        ])),
    );
    assert_eq!(usage.counts(), (0, 3));
    assert_eq!(usage.containers("shop", "web-0").len(), 1);
    assert_eq!(
        usage.pod("shop", "web-1").and_then(|p| p.memory_bytes),
        Some(80 * MI)
    );

    usage.apply(
        MetricsKind::Pods,
        at(15),
        &Ok(MetricsOutcome::Available(vec![pod_sample(
            "shop",
            "web-0",
            at(15),
            55,
            64,
        )])),
    );
    assert_eq!(usage.counts(), (0, 1), "deleted pods are evicted");
    assert_eq!(usage.pod("shop", "web-1"), None);
    assert!(usage.containers("kube-system", "dns").is_empty());
    let history: Vec<_> = usage
        .pod_history("shop", "web-0")
        .iter()
        .filter_map(|p| p.cpu_nanocores)
        .collect();
    assert_eq!(history, [50 * MILLI, 55 * MILLI]);
}

#[test]
fn absence_clears_the_samples_and_is_a_state_with_a_reason() {
    let usage = ClusterUsage::new();
    usage.apply(
        MetricsKind::Nodes,
        t0(),
        &Ok(nodes(vec![node_sample("a", t0(), 1, 1)])),
    );
    usage.apply(
        MetricsKind::Nodes,
        at(15),
        &Ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled)),
    );
    assert_eq!(usage.node("a"), None, "never a stale value under the badge");
    let state = usage.state(MetricsKind::Nodes);
    assert_eq!(
        state,
        MetricsState::Unavailable(MissingReason::NotInstalled)
    );
    let text = state.unavailable_text().expect("a reason");
    assert!(text.starts_with("metrics unavailable: "), "{text}");
    assert!(text.contains("metrics-server is not installed"), "{text}");

    let forbidden = MetricsState::Unavailable(MissingReason::Forbidden);
    assert!(
        forbidden
            .unavailable_text()
            .unwrap()
            .contains("no permission")
    );
    // The other kind is untouched.
    assert_eq!(usage.state(MetricsKind::Pods), MetricsState::Idle);
    assert_eq!(MetricsState::Idle.unavailable_text(), None);
    assert_eq!(MetricsState::Loading.unavailable_text(), None);
}

#[test]
fn a_failed_call_keeps_the_last_samples_and_says_why() {
    let usage = ClusterUsage::new();
    usage.apply(
        MetricsKind::Nodes,
        t0(),
        &Ok(nodes(vec![node_sample("a", t0(), 1, 1)])),
    );
    usage.apply(
        MetricsKind::Nodes,
        at(15),
        &Err(OxiError::timeout("the API server did not answer in 10s")),
    );
    assert!(usage.node("a").is_some());
    assert_eq!(
        usage
            .state(MetricsKind::Nodes)
            .unavailable_text()
            .as_deref(),
        Some("metrics unavailable: the API server did not answer in 10s")
    );
}

#[test]
fn clearing_forgets_everything() {
    let usage = ClusterUsage::new();
    usage.apply(
        MetricsKind::Nodes,
        t0(),
        &Ok(nodes(vec![node_sample("a", t0(), 1, 1)])),
    );
    usage.clear();
    assert_eq!(usage.counts(), (0, 0));
    assert_eq!(usage.state(MetricsKind::Nodes), MetricsState::Idle);
}
