//! The client against a real Prometheus running in the kind test cluster.
//!
//! A Prometheus pod (scraping itself every second) is created in a fresh `oxi-test-<rand>`
//! namespace, reached through `kubectl port-forward`, and queried with the client. Nothing is
//! installed outside that namespace; it is deleted when the test ends. Skips cleanly when
//! `OXIKUBE_TEST_CONTEXT` is unset.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use oxikube_ports::{PromEndpoint, PromqlEndpointPort, PromqlError, TimeRange};
use oxikube_prometheus::PromqlClient;
use oxikube_testkit::images;
use oxikube_testkit::integration::{TestNamespace, test_context};

fn manifest() -> String {
    format!(
        r#"apiVersion: v1
kind: ConfigMap
metadata:
  name: prom-config
data:
  prometheus.yml: |
    global:
      scrape_interval: 1s
    scrape_configs:
      - job_name: self
        static_configs:
          - targets: ['localhost:9090']
---
apiVersion: v1
kind: Pod
metadata:
  name: prom
spec:
  containers:
    - name: prometheus
      image: {image}
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
"#,
        image = images::PROMETHEUS
    )
}

fn kubectl(ns: &TestNamespace) -> Command {
    let mut cmd = Command::new("kubectl");
    cmd.args(["--context", ns.context(), "-n", ns.name()]);
    cmd
}

/// `kubectl port-forward` to the pod, killed on drop.
struct PortForward {
    child: Child,
    port: u16,
    /// Kept open: kubectl exits on SIGPIPE when it logs "Handling connection" to a closed pipe.
    _stdout: BufReader<ChildStdout>,
}

impl PortForward {
    fn start(ns: &TestNamespace) -> Self {
        let mut child = kubectl(ns)
            .args(["port-forward", "pod/prom", ":9090"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("kubectl port-forward");
        let stdout = child.stdout.take().unwrap();
        let mut stdout = BufReader::new(stdout);
        let mut line = String::new();
        stdout.read_line(&mut line).expect("port-forward output");
        // "Forwarding from 127.0.0.1:54321 -> 9090"
        let port = line
            .split("127.0.0.1:")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("unexpected port-forward output: {line}"));
        Self {
            child,
            port,
            _stdout: stdout,
        }
    }
}

impl Drop for PortForward {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn queries_a_real_prometheus_in_kind() {
    let Some(context) = test_context() else {
        return;
    };
    let ns = TestNamespace::create(&context).expect("namespace");

    let mut apply = kubectl(&ns)
        .args(["apply", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    apply
        .stdin
        .take()
        .unwrap()
        .write_all(manifest().as_bytes())
        .unwrap();
    assert!(apply.wait().unwrap().success(), "kubectl apply");

    // The image may need pulling on a cluster that has not pre-pulled it.
    let ready = kubectl(&ns)
        .args([
            "wait",
            "--for=condition=Ready",
            "pod/prom",
            "--timeout=240s",
        ])
        .output()
        .unwrap();
    assert!(
        ready.status.success(),
        "prometheus pod not ready: {}\n{}",
        String::from_utf8_lossy(&ready.stderr),
        ns.events()
    );

    let forward = PortForward::start(&ns);
    let endpoint = PromEndpoint::new(format!("http://127.0.0.1:{}", forward.port))
        .with_timeout(Duration::from_secs(10));
    let client = PromqlClient::new();

    // The forward and the server may need a moment after the pod reports ready.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match client.ping(&endpoint).await {
            Ok(()) => break,
            Err(e) if e.is_absent() && Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => panic!("ping: {e}"),
        }
    }

    // Wait for the first scrape to be queryable.
    let deadline = Instant::now() + Duration::from_secs(60);
    let up = loop {
        let series = client
            .instant_query(&endpoint, "up", None)
            .await
            .expect("instant query");
        if !series.is_empty() {
            break series;
        }
        assert!(Instant::now() < deadline, "no sample for `up` after 60s");
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    assert_eq!(up[0].labels["job"], "self");
    assert_eq!(up[0].points[0].value, 1.0);

    // Let a few more scrapes land, then ask for a range.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let end = Timestamp::now();
    let range = TimeRange {
        start: end - Duration::from_secs(10),
        end,
        step: Duration::from_secs(2),
    };
    let matrix = client
        .range_query(&endpoint, "up", range)
        .await
        .expect("range query");
    assert_eq!(matrix.len(), 1);
    assert!(matrix[0].points.len() >= 2, "{:?}", matrix[0].points);

    // A scalar, an empty result and a rejected query.
    let scalar = client
        .instant_query(&endpoint, "2 * 3", None)
        .await
        .unwrap();
    assert_eq!(scalar[0].points[0].value, 6.0);
    assert!(
        client
            .instant_query(&endpoint, "no_such_metric_oxikube", None)
            .await
            .unwrap()
            .is_empty()
    );
    let err = client
        .instant_query(&endpoint, "up{", None)
        .await
        .unwrap_err();
    assert!(matches!(err, PromqlError::BadQuery { .. }), "{err:?}");

    // The pod's own `/` path prefix handling: a wrong prefix is a 404 the UI can explain.
    let wrong = endpoint.clone().with_path_prefix("/not-here");
    let err = client.instant_query(&wrong, "up", None).await.unwrap_err();
    assert!(matches!(err, PromqlError::BadResponse(_)), "{err:?}");
}
