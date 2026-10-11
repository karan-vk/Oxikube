//! The catalogue against a real Prometheus in the kind test cluster.
//!
//! A Prometheus pod in a fresh `oxi-test-<rand>` namespace scrapes the node's kubelet cAdvisor
//! through the API server (the same series every Kubernetes stack scrapes), next to a busy pod.
//! The catalogue's queries for that pod run against it, so the cAdvisor-based queries are proven
//! against a real engine and real series: valid PromQL, and the numbers and labels a graph needs.
//! The queries over kube-state-metrics and node-exporter series (not installed here) must be
//! valid and simply answer nothing; they are covered by the nightly kube-prometheus-stack job
//! (E13-S13).
//!
//! The pod also needs a ClusterRole to read the kubelet; both are named after the namespace and
//! deleted when the test ends. Nothing else is created or changed. Skips cleanly when
//! `OXIKUBE_TEST_CONTEXT` is unset.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::forward::PortForward;
use jiff::Timestamp;
use oxikube_domain::{MetricKind, MetricRequest, MetricTarget, MetricsProviderKind, PodSelector};
use oxikube_ports::{
    PromEndpoint, PromqlEndpointPort, PromqlError, PromqlSeries, QueryCataloguePort, RangePlan,
};
use oxikube_prometheus::{PromqlCatalogue, PromqlClient};
use oxikube_testkit::images;
use oxikube_testkit::integration::{TestNamespace, test_context};

/// A window of six 5-second scrapes: wide enough to hold two samples, short enough to wait for.
const WINDOW: Duration = Duration::from_secs(30);

fn manifests(ns: &str, role: &str) -> String {
    format!(
        r#"apiVersion: v1
kind: ServiceAccount
metadata:
  name: prom
  namespace: {ns}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: {role}
  labels:
    oxikube-test: {ns}
rules:
  - apiGroups: [""]
    resources: ["nodes", "nodes/proxy", "nodes/metrics"]
    verbs: ["get", "list", "watch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: {role}
  labels:
    oxikube-test: {ns}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: {role}
subjects:
  - kind: ServiceAccount
    name: prom
    namespace: {ns}
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: prom-config
  namespace: {ns}
data:
  prometheus.yml: |
    global:
      scrape_interval: 5s
    scrape_configs:
      - job_name: cadvisor
        scheme: https
        authorization:
          credentials_file: /var/run/secrets/kubernetes.io/serviceaccount/token
        tls_config:
          ca_file: /var/run/secrets/kubernetes.io/serviceaccount/ca.crt
        kubernetes_sd_configs:
          - role: node
        relabel_configs:
          - target_label: __address__
            replacement: kubernetes.default.svc:443
          - source_labels: [__meta_kubernetes_node_name]
            target_label: __metrics_path__
            replacement: /api/v1/nodes/$1/proxy/metrics/cadvisor
          - source_labels: [__meta_kubernetes_node_name]
            target_label: node
---
apiVersion: v1
kind: Pod
metadata:
  name: prom
  namespace: {ns}
spec:
  serviceAccountName: prom
  containers:
    - name: prometheus
      image: {prometheus}
      args: ["--config.file=/etc/prom/prometheus.yml", "--storage.tsdb.path=/tmp/data"]
      ports:
        - containerPort: 9090
      readinessProbe:
        httpGet:
          path: /-/ready
          port: 9090
        periodSeconds: 1
      volumeMounts:
        - name: config
          mountPath: /etc/prom
  volumes:
    - name: config
      configMap:
        name: prom-config
---
apiVersion: v1
kind: Pod
metadata:
  name: spin
  namespace: {ns}
spec:
  containers:
    - name: burner
      image: {busybox}
      command: ["sh", "-c", "while true; do :; done"]
      resources:
        requests: {{cpu: 10m, memory: 8Mi}}
        limits: {{cpu: 200m, memory: 64Mi}}
    - name: idle
      image: {busybox}
      command: ["sleep", "3600"]
"#,
        prometheus = images::PROMETHEUS,
        busybox = images::BUSYBOX,
    )
}

/// Deletes the cluster-scoped objects of this test when dropped (the namespace goes with its
/// own guard).
struct ClusterScoped<'a> {
    context: &'a str,
    ns: &'a str,
}

impl Drop for ClusterScoped<'_> {
    fn drop(&mut self) {
        let _ = Command::new("kubectl")
            .args(["--context", self.context])
            .args(["delete", "clusterrole,clusterrolebinding", "-l"])
            .arg(format!("oxikube-test={}", self.ns))
            .args(["--wait=false"])
            .output();
    }
}

fn kubectl(ns: &TestNamespace, args: &[&str]) -> std::process::Output {
    Command::new("kubectl")
        .args(["--context", ns.context(), "-n", ns.name()])
        .args(args)
        .output()
        .expect("kubectl")
}

async fn instant(
    client: &PromqlClient,
    endpoint: &PromEndpoint,
    promql: &str,
) -> Result<Vec<PromqlSeries>, PromqlError> {
    client.instant_query(endpoint, promql, None).await
}

/// Re-runs `promql` until it answers at least one series, or panics at the deadline.
async fn eventually(
    client: &PromqlClient,
    endpoint: &PromEndpoint,
    promql: &str,
    what: &str,
) -> Vec<PromqlSeries> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let series = instant(client, endpoint, promql)
            .await
            .unwrap_or_else(|e| panic!("{what}: {e}\n{promql}"));
        if !series.is_empty() {
            return series;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: no series after 120s\n{promql}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[tokio::test]
async fn the_catalogue_runs_against_a_real_prometheus_in_kind() {
    let Some(context) = test_context() else {
        return;
    };
    let ns = TestNamespace::create(&context).expect("namespace");
    let _cleanup = ClusterScoped {
        context: &context,
        ns: ns.name(),
    };
    let role = format!("{}-prom", ns.name());

    let mut apply = Command::new("kubectl")
        .args(["--context", &context, "apply", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    apply
        .stdin
        .take()
        .unwrap()
        .write_all(manifests(ns.name(), &role).as_bytes())
        .unwrap();
    assert!(apply.wait().unwrap().success(), "kubectl apply");

    for pod in ["prom", "spin"] {
        let ready = kubectl(
            &ns,
            &[
                "wait",
                "--for=condition=Ready",
                &format!("pod/{pod}"),
                "--timeout=240s",
            ],
        );
        assert!(
            ready.status.success(),
            "{pod} not ready: {}\n{}",
            String::from_utf8_lossy(&ready.stderr),
            ns.events()
        );
    }

    let forward = PortForward::start(&context, ns.name(), "prom", 9090);
    let endpoint = PromEndpoint::new(format!("http://127.0.0.1:{}", forward.port))
        .with_timeout(Duration::from_secs(15));
    let client = PromqlClient::new();
    let provider = MetricsProviderKind::KubePrometheus;
    let catalogue = PromqlCatalogue;
    let me = ns.name().to_owned();

    let build = |target: MetricTarget, kind: MetricKind| {
        catalogue
            .build(provider, &MetricRequest::new(target, kind), WINDOW)
            .unwrap()
    };

    // The cAdvisor series arrive after the first scrape of the kubelet.
    let pod = |name: &str| MetricTarget::Pod {
        namespace: me.clone(),
        pods: PodSelector::One(name.into()),
    };
    let containers = MetricTarget::Container {
        namespace: me.clone(),
        pod: "spin".into(),
        container: None,
    };

    // Memory first: a gauge, present from the first scrape.
    let memory = build(containers.clone(), MetricKind::MemoryUsage);
    assert_eq!(memory.series_label, Some("container"));
    let series = eventually(&client, &endpoint, &memory.promql, "container memory").await;
    let mut names: Vec<_> = series
        .iter()
        .map(|s| s.labels["container"].clone())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["burner", "idle"],
        "one series per container, pause excluded"
    );
    assert!(series.iter().all(|s| s.points[0].value > 0.0));

    // CPU: the burner spins at its 200m limit, the idle container does not.
    let cpu = build(containers, MetricKind::CpuUsage);
    let deadline = Instant::now() + Duration::from_secs(120);
    let burner = loop {
        let series = eventually(&client, &endpoint, &cpu.promql, "container cpu").await;
        let burner = series
            .iter()
            .find(|s| s.labels["container"] == "burner")
            .unwrap();
        if burner.points[0].value > 0.05 {
            break burner.points[0].value;
        }
        assert!(
            Instant::now() < deadline,
            "burner never used CPU: {series:?}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    assert!(burner <= 0.45, "a 200m limit, got {burner} cores");

    // The pod total and the namespace total agree with the containers.
    let pod_cpu = eventually(
        &client,
        &endpoint,
        &build(pod("spin"), MetricKind::CpuUsage).promql,
        "pod cpu",
    )
    .await;
    assert_eq!(pod_cpu.len(), 1);
    assert_eq!(pod_cpu[0].labels["pod"], "spin");
    let ns_cpu = eventually(
        &client,
        &endpoint,
        &build(MetricTarget::Namespace(me.clone()), MetricKind::CpuUsage).promql,
        "namespace cpu",
    )
    .await;
    assert_eq!(ns_cpu.len(), 1);
    assert!(ns_cpu[0].labels.is_empty());
    assert!(ns_cpu[0].points[0].value >= pod_cpu[0].points[0].value * 0.5);

    // Network is counted per pod, including the pod that does nothing.
    let rx = eventually(
        &client,
        &endpoint,
        &build(pod("spin"), MetricKind::NetworkRx).promql,
        "pod rx",
    )
    .await;
    assert_eq!(rx[0].labels["pod"], "spin");

    // A range query over the plan's step and window returns a line.
    let plan = RangePlan::ending_at(Timestamp::now(), Duration::from_secs(15 * 60));
    let ranged = catalogue
        .build(
            provider,
            &MetricRequest::new(pod("spin"), MetricKind::MemoryUsage),
            plan.rate_window,
        )
        .unwrap();
    let matrix = client
        .range_query(&endpoint, &ranged.promql, plan.range)
        .await
        .unwrap();
    assert_eq!(matrix.len(), 1);
    assert_eq!(matrix[0].labels["pod"], "spin");
    assert!(!matrix[0].points.is_empty());

    // Every supported query of every provider is valid PromQL for a real engine, including with
    // names that try to break out of their string. Series that this cluster does not have answer
    // nothing; an unknown selector is not an error and a malformed query is `BadQuery`.
    let hostile = [r#"a"} or vector(1) #"#, r"b\", "c\nd", ".*"];
    let mut ran = 0;
    for provider in MetricsProviderKind::ALL {
        for kind in MetricKind::ALL {
            let mut targets = vec![
                MetricTarget::Cluster,
                MetricTarget::Node("kind-control-plane".into()),
                MetricTarget::Namespace(me.clone()),
                pod("spin"),
                MetricTarget::Container {
                    namespace: me.clone(),
                    pod: "spin".into(),
                    container: Some("burner".into()),
                },
                MetricTarget::Pvc {
                    namespace: me.clone(),
                    name: "data".into(),
                },
                MetricTarget::Ingress {
                    namespace: me.clone(),
                    name: "web".into(),
                },
            ];
            for name in hostile {
                targets.push(MetricTarget::Namespace(name.into()));
                targets.push(MetricTarget::Pod {
                    namespace: name.into(),
                    pods: PodSelector::Any(vec![name.into(), "x".into()]),
                });
                targets.push(MetricTarget::Node(name.into()));
                targets.push(MetricTarget::Ingress {
                    namespace: name.into(),
                    name: name.into(),
                });
            }
            for target in targets {
                let request = MetricRequest::new(target, kind);
                let Ok(query) = catalogue.build(provider, &request, WINDOW) else {
                    continue;
                };
                if let Err(e) = instant(&client, &endpoint, &query.promql).await {
                    panic!("{provider} {request:?}: {e}\n{}", query.promql);
                }
                ran += 1;
            }
        }
    }
    assert!(ran > 500, "ran {ran}");

    // A hostile namespace matches no series of the real data: it did not widen the selector.
    let widened = build(
        MetricTarget::Namespace(".*".into()),
        MetricKind::MemoryUsage,
    );
    assert!(
        instant(&client, &endpoint, &widened.promql)
            .await
            .unwrap()
            .is_empty()
    );
}
