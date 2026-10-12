//! The allocation views (E13-S11) against the real cluster: the numbers the Node and Namespace
//! panels show are compared with what the API server and `kubectl` say.
//!
//! * A namespace with a `ResourceQuota` and two pods: the quota controller keeps `status.used`,
//!   so the S02 sums over the listed pods must equal it for requests, limits and the pod count,
//!   and the view measures them against the quota's `hard`.
//! * The real node: requests and limits over the pods that `spec.nodeName` places on it equal the
//!   "Allocated resources" block of `kubectl describe node`.
//! * Usage through the real metrics adapter: a reading when metrics-server answers, otherwise a
//!   `metrics unavailable: <reason>` state, never a zero.
//!
//! Everything lives in the test's own `oxi-test-<rand>` namespace; the node is only read.

use std::process::Command as Process;
use std::time::Duration;

use k8s_openapi::api::core::v1::{Pod, ResourceQuota};
use kube::Api;
use kube::api::PostParams;
use oxikube_app::allocation::{
    AllocationKind, AllocationView, Usage, UsageState, namespace_usage, node_usage,
};
use oxikube_app::nodes::list_node_pods;
use oxikube_app::{allocate_namespaces, allocate_nodes};
use oxikube_domain::Quantity;
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::utilisation::{GpuVendors, QuotaCaps};
use oxikube_kube::{KubeDiscovery, KubeMetrics, KubeResources};
use oxikube_ports::{ListOptions, ResourceReader};
use oxikube_testkit::integration::TestNamespace;
use serde_json::json;

use crate::cluster::Kind;

fn q(text: &str) -> Quantity {
    Quantity::parse(text).expect("a quantity")
}

async fn pod_with_resources(
    client: &kube::Client,
    namespace: &str,
    name: &str,
    (cpu, limit): (&str, &str),
) {
    let pod: Pod = serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name},
        "spec": {
            // A scheduler nobody runs: the pod stays Pending (the quota counts it all the same).
            "schedulerName": "oxikube-test-no-scheduler",
            "terminationGracePeriodSeconds": 1,
            "containers": [{
                "name": "pause",
                "image": oxikube_testkit::images::PAUSE,
                "resources": {
                    "requests": {"cpu": cpu, "memory": "64Mi"},
                    "limits": {"cpu": limit, "memory": "128Mi"}
                }
            }]
        }
    }))
    .expect("pod json");
    Api::<Pod>::namespaced(client.clone(), namespace)
        .create(&PostParams::default(), &pod)
        .await
        .expect("create a pod");
}

async fn list(reader: &dyn ResourceReader, kind: &Gvk, namespace: Option<&str>) -> Vec<Resource> {
    reader
        .list(kind, namespace, &ListOptions::default())
        .await
        .expect("list")
        .items
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_namespace_against_its_quota_matches_what_the_api_server_counts() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let cluster = ClusterId::new("kind-smoke", &kind.context);
    let quota: ResourceQuota = serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "ResourceQuota",
        "metadata": {"name": "compute"},
        "spec": {"hard": {
            "requests.cpu": "2", "limits.cpu": "4", "requests.memory": "1Gi",
            "limits.memory": "2Gi", "pods": "5",
        }},
    }))
    .expect("quota json");
    let quotas = Api::<ResourceQuota>::namespaced(client.clone(), ns.name());
    quotas
        .create(&PostParams::default(), &quota)
        .await
        .expect("create the quota");
    // Admission charges a pod only to a quota whose status the controller has already filled in,
    // and the controller recounts on pod changes, not on pod creation: a pod created before then
    // waits for the full resync (minutes). So the pods come once the quota counts.
    crate::eventually(
        "the quota controller to publish the quota's status",
        String::new,
        || async {
            quotas
                .get("compute")
                .await
                .expect("get the quota")
                .status
                .and_then(|status| status.used)
                .is_some_and(|used| used.contains_key("pods"))
        },
    )
    .await;
    pod_with_resources(&client, ns.name(), "a", ("250m", "500m")).await;
    pod_with_resources(&client, ns.name(), "b", ("750m", "1")).await;

    // The quota controller's own accounting is the oracle.
    let read_used = || async {
        quotas
            .get("compute")
            .await
            .expect("get the quota")
            .status
            .and_then(|status| status.used)
            .unwrap_or_default()
    };
    crate::eventually(
        "the quota controller to count both pods",
        String::new,
        || async {
            read_used()
                .await
                .get("pods")
                .is_some_and(|pods| pods.0 == "2")
        },
    )
    .await;
    let used = read_used().await;

    let reader = KubeResources::new(client.clone(), KubeDiscovery::new(client.clone()));
    let pods = list(&reader, &Gvk::new("", "v1", "Pod"), Some(ns.name())).await;
    let quotas = list(
        &reader,
        &Gvk::new("", "v1", "ResourceQuota"),
        Some(ns.name()),
    )
    .await;
    let gpus = GpuVendors::default();
    let sums = allocate_namespaces(pods.iter(), &gpus);
    let namespace = sums
        .iter()
        .find(|a| &*a.namespace == ns.name())
        .expect("the namespace has pods");
    let (caps, scoped) = QuotaCaps::tightest(quotas.iter(), &gpus);
    assert_eq!(scoped, 0);

    let metrics = KubeMetrics::new(client.clone(), cluster.clone());
    let usage = namespace_usage(&metrics, &cluster, ns.name()).await;
    let view = AllocationView::for_namespace(namespace, &caps, scoped, &usage);

    let server = |key: &str| q(&used.get(key).expect("the quota counts it").0);
    let row = |kind: AllocationKind| view.row(&kind).expect("a row").clone();
    let cpu = row(AllocationKind::Cpu);
    let memory = row(AllocationKind::Memory);
    assert_eq!(cpu.requests, server("requests.cpu"), "1 CPU in requests");
    assert_eq!(cpu.limits, Some(server("limits.cpu")), "500m + 1 in limits");
    assert_eq!(memory.requests, server("requests.memory"));
    assert_eq!(memory.limits, Some(server("limits.memory")));
    let pods_row = row(AllocationKind::Pods);
    assert_eq!(pods_row.requests, server("pods"));

    // Measured against the quota's hard amounts.
    assert_eq!(cpu.capacity, Some(q("2")));
    assert_eq!(cpu.limits_capacity, Some(q("4")));
    assert_eq!(cpu.requests_text(), "1000m of 2000m (50%)");
    assert_eq!(cpu.limits_text().as_deref(), Some("1500m of 4000m (37%)"));
    assert_eq!(pods_row.requests_text(), "2 of 5 (40%)");

    // The pods are Pending: metrics-server has no reading for them, and the view says so rather
    // than showing a zero (or, with no metrics-server at all, why).
    match usage {
        UsageState::Ready(measured) => {
            assert_eq!(measured.pods_reporting, 0, "pending pods have no samples");
            assert!(
                matches!(&cpu.usage, Usage::Unavailable(text)
                if text.starts_with("metrics unavailable: ")),
                "{:?}",
                cpu.usage
            );
        }
        UsageState::Unavailable(text) => {
            assert!(text.starts_with("metrics unavailable: "), "{text}")
        }
        UsageState::Pending => panic!("a call that returned is not pending"),
    }
}

/// `kubectl describe node`'s "Allocated resources": `(cpu requests, cpu limits, memory requests,
/// memory limits)` as printed.
fn described(context: &str, node: &str) -> Option<[String; 4]> {
    let out = Process::new("kubectl")
        .args(["--context", context, "describe", "node", node])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let block = text.split("Allocated resources:").nth(1)?;
    let line = |resource: &str| -> Option<(String, String)> {
        let row = block
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(resource) && l[resource.len()..].starts_with(' '))?;
        let cells: Vec<&str> = row.split_whitespace().collect();
        Some((cells.get(1)?.to_string(), cells.get(3)?.to_string()))
    };
    let (cpu_requests, cpu_limits) = line("cpu")?;
    let (memory_requests, memory_limits) = line("memory")?;
    Some([cpu_requests, cpu_limits, memory_requests, memory_limits])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_real_node_matches_kubectl_describe_and_has_a_usage_reading_or_a_reason() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let cluster = ClusterId::new("kind-smoke", &kind.context);
    let name = crate::nodes::real_node(&client)
        .await
        .metadata
        .name
        .expect("a node name");
    let reader = KubeResources::new(client.clone(), KubeDiscovery::new(client.clone()));
    let gpus = GpuVendors::default();

    // Other suites start and stop pods while this one reads: compare two reads taken together,
    // and try again when the pods moved in between.
    let mut last = String::new();
    let mut compared = false;
    for _ in 0..10 {
        let node = reader
            .get(&Gvk::new("", "v1", "Node"), None, &name)
            .await
            .expect("get the node");
        let pods = list_node_pods(&reader, &name)
            .await
            .expect("pods on the node");
        let Some([cpu_r, cpu_l, mem_r, mem_l]) = described(kind.context.as_str(), &name) else {
            panic!("kubectl describe node printed no allocated resources");
        };
        let all = allocate_nodes([&node], pods.pods.iter().map(|p| &**p), &gpus);
        let view = AllocationView::for_node(&all[0], &UsageState::Pending);
        let cpu = view.row(&AllocationKind::Cpu).unwrap();
        let memory = view.row(&AllocationKind::Memory).unwrap();
        let same_memory = |ours: Quantity, printed: &str| {
            // `kubectl` prints memory rounded to a whole unit of its own choosing.
            (ours.value() - q(printed).value()).abs() < (1 << 20)
        };
        let same = cpu.requests == q(&cpu_r)
            && cpu.limits == Some(q(&cpu_l))
            && same_memory(memory.requests, &mem_r)
            && same_memory(memory.limits.unwrap(), &mem_l);
        if same {
            compared = true;
            break;
        }
        last = format!(
            "ours: cpu {} / {:?}, memory {} / {:?}; kubectl: cpu {cpu_r} / {cpu_l}, memory {mem_r} / {mem_l}",
            cpu.requests_text(),
            cpu.limits_text(),
            memory.requests_text(),
            memory.limits_text()
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        compared,
        "the node's allocation never matched kubectl describe:\n{last}"
    );

    // Usage through the real adapter: a reading, or a reason.
    let metrics = KubeMetrics::new(client.clone(), cluster.clone());
    match node_usage(&metrics, &cluster, &name).await {
        UsageState::Ready(measured) => {
            let cpu = measured.cpu.expect("a cpu reading");
            let memory = measured.memory.expect("a memory reading");
            assert!(!memory.is_zero(), "a running node uses memory");
            assert!(!cpu.is_negative());
        }
        UsageState::Unavailable(text) => {
            assert!(text.starts_with("metrics unavailable: "), "{text}");
        }
        UsageState::Pending => panic!("a call that returned is not pending"),
    }
}
