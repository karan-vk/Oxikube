//! Usage text to samples: Ki/Mi/Gi, `m`, `u` and `n` suffixes and the forms kdash gets wrong.

use jiff::Timestamp;
use oxikube_domain::metrics::{MetricsSubject, MissingReason, Reading};

use super::cluster;
use crate::metrics::convert::{node_sample, pod_sample};
use crate::metrics::raw::{RawNode, RawPod, RawUsage};

fn now() -> Timestamp {
    "2026-10-04T00:00:00Z".parse().unwrap()
}

fn usage(cpu: &str, memory: &str) -> RawUsage {
    RawUsage {
        cpu: Some(cpu.to_owned()),
        memory: Some(memory.to_owned()),
    }
}

fn node(cpu: &str, memory: &str) -> RawNode {
    RawNode {
        name: "worker-1".into(),
        timestamp: Some("2026-10-03T12:00:00Z".parse().unwrap()),
        window: Some(jiff::SignedDuration::from_millis(14_982)),
        usage: usage(cpu, memory),
    }
}

fn pod(containers: Vec<RawUsage>) -> RawPod {
    RawPod {
        namespace: "kube-system".into(),
        name: "coredns-0".into(),
        timestamp: Some("2026-10-03T12:00:01Z".parse().unwrap()),
        window: None,
        containers,
    }
}

#[test]
fn cpu_suffixes_become_exact_nanocores() {
    for (text, nanos) in [
        ("6082165n", 6_082_165),
        ("303u", 303_000),
        ("3491m", 3_491_000_000),
        ("2", 2_000_000_000),
        ("1.5", 1_500_000_000),
        ("0", 0),
        ("1e-3", 1_000_000),
    ] {
        let sample = node_sample(node(text, "1Ki"), now());
        assert_eq!(sample.cpu, Reading::Value(nanos), "cpu {text}");
    }
}

#[test]
fn memory_suffixes_become_exact_bytes() {
    for (text, bytes) in [
        ("22272Ki", 22_806_528),
        ("5Mi", 5_242_880),
        ("3Gi", 3_221_225_472),
        ("1.5Gi", 1_610_612_736),
        ("129e6", 129_000_000),
        ("128974848", 128_974_848),
        ("129M", 129_000_000),
        ("1k", 1000),
        ("0004Ki", 4096),
        ("1Ti", 1 << 40),
    ] {
        let sample = node_sample(node("1", text), now());
        assert_eq!(sample.memory, Reading::Value(bytes), "memory {text}");
    }
}

#[test]
fn node_sample_carries_the_server_timestamp_and_window() {
    let sample = node_sample(node("196382978n", "1848836Ki"), now());
    assert_eq!(
        sample.subject,
        MetricsSubject::Node {
            node: "worker-1".into()
        }
    );
    assert_eq!(
        sample.ts,
        "2026-10-03T12:00:00Z".parse::<Timestamp>().unwrap()
    );
    assert_eq!(
        sample.window,
        Some(jiff::SignedDuration::from_millis(14_982))
    );
    assert_eq!(sample.cpu_millicores(), Some(196));
    assert_eq!(sample.memory_bytes(), Some(1_893_208_064));
}

#[test]
fn a_missing_server_timestamp_falls_back_to_the_fetch_time() {
    let mut raw = node("1", "1Ki");
    raw.timestamp = None;
    assert_eq!(node_sample(raw, now()).ts, now());
}

#[test]
fn pod_usage_is_the_exact_sum_over_containers() {
    let sample = pod_sample(
        &cluster(),
        pod(vec![
            usage("100m", "10Mi"),
            usage("250500u", "1Gi"),
            usage("1n", "1"),
        ]),
        now(),
    );
    // 100m + 250.5m + 1n = 350_500_001 nanocores; no float rounding.
    assert_eq!(sample.cpu, Reading::Value(350_500_001));
    assert_eq!(
        sample.memory,
        Reading::Value(10 * 1024 * 1024 + (1 << 30) + 1)
    );
    let MetricsSubject::Pod { pod } = sample.subject else {
        panic!("a pod subject")
    };
    assert_eq!(pod.namespace(), Some("kube-system"));
    assert_eq!(&*pod.name, "coredns-0");
    assert_eq!(&*pod.gvk.kind, "Pod");
}

#[test]
fn a_pod_without_scraped_containers_is_not_yet_available() {
    let sample = pod_sample(&cluster(), pod(Vec::new()), now());
    let reason = Reading::Missing(MissingReason::NotYetAvailable);
    assert_eq!((sample.cpu, sample.memory), (reason, reason));
}

#[test]
fn an_unreadable_quantity_marks_only_that_reading_missing() {
    let sample = node_sample(node("12 cores", "64Mi"), now());
    assert_eq!(sample.cpu, Reading::Missing(MissingReason::Unavailable));
    assert_eq!(sample.memory, Reading::Value(64 * 1024 * 1024));
    assert!(sample.is_partial());

    // One bad container makes the pod's sum unknown rather than understated.
    let sample = pod_sample(
        &cluster(),
        pod(vec![usage("100m", "1Mi"), usage("lots", "1Mi")]),
        now(),
    );
    assert_eq!(sample.cpu, Reading::Missing(MissingReason::Unavailable));
    assert_eq!(sample.memory, Reading::Value(2 * 1024 * 1024));
}

#[test]
fn absent_and_negative_usage_are_missing_not_zero() {
    let mut raw = node("1", "1Ki");
    raw.usage.memory = None;
    assert_eq!(
        node_sample(raw, now()).memory,
        Reading::Missing(MissingReason::Unavailable)
    );
    let sample = node_sample(node("-5m", "-1Ki"), now());
    assert_eq!(sample.cpu, Reading::Missing(MissingReason::Unavailable));
    assert_eq!(sample.memory, Reading::Missing(MissingReason::Unavailable));
}

#[test]
fn utilisation_uses_domain_quantity_maths() {
    use oxikube_domain::Quantity;
    let sample = node_sample(node("500m", "512Mi"), now());
    assert_eq!(
        sample.cpu_percent_of(&Quantity::parse("2").unwrap()),
        Some(25.0)
    );
    assert_eq!(
        sample.memory_percent_of(&Quantity::parse("2Gi").unwrap()),
        Some(25.0)
    );
}
