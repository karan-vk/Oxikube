//! History metrics against a real cluster (E13-S05): a Prometheus pod and Service in a namespace of
//! the test's own, reached through the real API server's service proxy.
//!
//! * The detector (the real `ProviderDetector` over the real Service list and service proxy
//!   adapters) finds the Service from a catalogue entry for the test namespace, classifies it,
//!   confirms it with a probe query, and the PromQL port then queries it.
//! * The app (the real init order, its main window headless, kind connected) checks a Service the
//!   `prometheus` settings name, shows it on the metrics page, answers queries through it, and
//!   ends the automatic detection of the shared cluster in a visible verdict with the services it
//!   looked at.
//!
//! Nothing is installed outside the test's `oxi-test-<rand>` namespace, which is deleted at the end;
//! the stacks' own namespaces (`monitoring`, ...) are only listed, never written.
//!
//! `cargo test -p oxikube --features integration --test kind_metrics` with `OXIKUBE_TEST_CONTEXT`
//! set (`cargo xtask kind-up`); without it the tests return at once. Real I/O wakes GPUI tasks from
//! Tokio threads, so the app tests allow parking and poll with short real-time sleeps.
#![cfg(feature = "integration")]

mod kind_common;

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::TestAppContext;
use oxikube::app_state::AppState;
use oxikube_app::ProviderState;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::MetricsProviderKind;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command as BusCommand;
use oxikube_domain::ids::ClusterId;
use oxikube_kube::{KubeDiscovery, KubeResources, KubeServiceProxy};
use oxikube_ports::{DetectionState, ProbeOutcome, ProviderDetectorPort, ProviderTarget};
use oxikube_prometheus::{Catalogue, ProviderDetector};
use oxikube_settings_ui::metrics::MetricsSettingsView;
use oxikube_testkit::images;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};

use std::future::Future;

use kind_common::{Launched, launch, launch_with, on_cluster, wait, wait_value};

/// A Prometheus that scrapes itself every second, and a Service `prom` in front of it.
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
  labels:
    app: prom
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
---
apiVersion: v1
kind: Service
metadata:
  name: prom
spec:
  selector:
    app: prom
  ports:
    - name: web
      port: 9090
      targetPort: 9090
"#,
        image = images::PROMETHEUS
    )
}

/// Applies the Prometheus pod and Service to `ns` and waits for the pod to be ready.
fn deploy_prometheus(ns: &TestNamespace) {
    let kubectl = || {
        let mut cmd = Command::new("kubectl");
        cmd.args(["--context", ns.context(), "-n", ns.name()]);
        cmd
    };
    let mut apply = kubectl()
        .args(["apply", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("kubectl apply");
    apply
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(manifest().as_bytes())
        .expect("write the manifest to kubectl apply");
    assert!(
        apply.wait().expect("wait for kubectl apply").success(),
        "kubectl apply"
    );
    // The image may need pulling on a cluster that has not pre-pulled it.
    let ready = kubectl()
        .args([
            "wait",
            "--for=condition=Ready",
            "pod/prom",
            "--timeout=240s",
        ])
        .output()
        .expect("run kubectl wait");
    assert!(
        ready.status.success(),
        "prometheus pod not ready: {}\n{}",
        String::from_utf8_lossy(&ready.stderr),
        ns.events()
    );
}

/// The catalogue the adapter test uses: the test namespace under a stack's service name.
fn catalogue_for(namespace: &str) -> Catalogue {
    Catalogue::parse(&format!(
        r#"{{ "entries": [
            {{ "kind": "mimir", "service": "mimir-query-frontend",
               "namespaces": ["{namespace}"], "ports": ["http-metrics"], "path_prefixes": ["/prometheus"] }},
            {{ "kind": "victoria_metrics", "service_prefix": "vmsingle",
               "namespaces": ["{namespace}"], "ports": ["http"] }},
            {{ "kind": "kube_prometheus", "service": "prom",
               "namespaces": ["{namespace}"], "ports": ["web", "9090"] }}
        ] }}"#
    ))
    .expect("a catalogue")
}

#[test]
fn the_detector_finds_a_prometheus_service_through_the_real_service_proxy() {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let ns = TestNamespace::create(&context).expect("namespace");
    deploy_prometheus(&ns);

    let namespace = ns.name().to_owned();
    let result = on_cluster(&context, async |client| {
        let discovery = KubeDiscovery::new(client.clone());
        let resources = Arc::new(KubeResources::new(client.clone(), discovery));
        let proxy = Arc::new(KubeServiceProxy::new(client));
        let detector = ProviderDetector::new().with_catalogue(catalogue_for(&namespace));
        // The pod is ready, but the endpoint may need a moment to reach the API server's view.
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let result = detector.detect(resources.clone(), proxy.clone()).await;
            if matches!(result.state, DetectionState::Found(_)) || Instant::now() > deadline {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    let DetectionState::Found(provider) = &result.state else {
        panic!("no provider found: {:?} {:?}", result.state, result.probes);
    };
    assert_eq!(provider.kind, MetricsProviderKind::KubePrometheus);
    let ProviderTarget::Service(service) = &provider.target else {
        panic!("{provider:?}");
    };
    assert_eq!(service.short(), format!("{namespace}/prom:web"));
    // Catalogue order: nothing of the first two entries exists, the third answered.
    let outcomes: Vec<_> = result.probes.iter().map(|p| p.outcome.clone()).collect();
    assert_eq!(
        outcomes,
        [
            ProbeOutcome::NotPresent,
            ProbeOutcome::NotPresent,
            ProbeOutcome::Answered
        ],
        "{:?}",
        result.probes
    );
}

fn status_of(
    vcx: &mut gpui::VisualTestContext,
    cluster: &ClusterId,
) -> Arc<oxikube_app::ProviderStatus> {
    vcx.update(|_, cx| {
        AppState::global(cx)
            .services()
            .metrics
            .provider
            .status(cluster)
    })
}

/// Runs `fut` on the app's Tokio runtime and waits for it (the test thread is the app's).
fn on_app_runtime<T>(vcx: &mut gpui::VisualTestContext, fut: impl Future<Output = T>) -> T {
    let handle = vcx
        .update(|_, cx| oxikube_runtime::handle(cx))
        .expect("the app runs on Tokio");
    handle.block_on(fut)
}

fn drawn(vcx: &mut gpui::VisualTestContext, selector: &'static str) -> bool {
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.debug_bounds(selector).is_some()
}

#[gpui::test]
fn the_app_checks_the_service_the_settings_name_queries_through_it_and_shows_it(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();
    let ns = TestNamespace::create(&context).expect("namespace");
    deploy_prometheus(&ns);

    let settings = serde_json::json!({
        "prometheus": { "path": format!("{}/prom:9090", ns.name()), "kind": "kube_prometheus" }
    });
    let Launched {
        mut vcx,
        cluster,
        tab,
        ..
    } = launch_with(cx, &context, settings);

    let status = wait_value(&mut vcx, "the configured service to answer", |vcx| {
        let status = status_of(vcx, &cluster);
        status.provider().is_some().then_some(status)
    });
    let provider = status.provider().unwrap();
    assert_eq!(provider.kind, MetricsProviderKind::KubePrometheus);
    assert_eq!(
        provider.target.to_string(),
        format!("{}/prom:9090", ns.name())
    );

    // The PromQL port reaches it through the same connection, and the first scrape lands.
    let promql = vcx.update(|_, cx| AppState::global(cx).services().metrics.promql.clone());
    let started = Instant::now();
    let up = loop {
        let series = on_app_runtime(&mut vcx, promql.query_instant(&cluster, "up", None))
            .expect("a query through the API server's service proxy");
        if !series.is_empty() {
            break series;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "no sample for `up`"
        );
        std::thread::sleep(Duration::from_millis(500));
    };
    assert_eq!(up[0].labels["job"], "self");

    // The page shows it, opened by the command (the sidebar's entry sends the same one).
    let state = vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let shown = futures::executor::block_on(bus.dispatch(
        BusCommand::ClusterShowMetricsSettings {
            cluster: cluster.clone(),
        },
        DispatchContext::new(Initiator::Ui, "test"),
    ));
    assert!(shown.is_ok());
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    wait(
        &mut vcx,
        "the metrics page to open in the cluster tab",
        |vcx| {
            vcx.update(|_, cx| {
                !inner
                    .read(cx)
                    .items_of_type::<MetricsSettingsView>()
                    .is_empty()
            })
        },
    );
    assert!(drawn(&mut vcx, "metrics-headline:good"));
    assert!(drawn(&mut vcx, "metrics-setting-Service"));
}

#[gpui::test]
fn automatic_detection_in_the_app_ends_in_a_visible_verdict_with_the_services_looked_at(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();
    let Launched {
        mut vcx, cluster, ..
    } = launch(cx, &context);

    let status = wait_value(&mut vcx, "detection to reach a verdict", |vcx| {
        let status = status_of(vcx, &cluster);
        matches!(status.state, ProviderState::Detection(ref d) if *d != DetectionState::Detecting)
            .then_some(status)
    });
    // The shared test cluster may or may not run a monitoring stack: either way the answer is
    // visible, and it names what was looked at. The test user lists services everywhere.
    assert!(
        !status.probes.is_empty(),
        "the services looked at are listed"
    );
    assert!(
        status
            .probes
            .iter()
            .all(|p| !matches!(p.outcome, ProbeOutcome::Skipped(_))),
        "{:?}",
        status.probes
    );
    match &status.state {
        ProviderState::Detection(DetectionState::Found(provider)) => {
            assert!(status.unavailable_text().is_none(), "{provider:?}");
        }
        ProviderState::Detection(DetectionState::NotFound) => {
            let text = status.unavailable_text().expect("absence has a reason");
            assert!(text.starts_with("metrics unavailable: "), "{text}");
        }
        other => panic!("detection against the kind cluster ended in {other:?}"),
    }

    // Looking again runs it again, off the UI thread, and ends in a verdict again.
    let state = vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let again = futures::executor::block_on(bus.dispatch(
        BusCommand::ClusterRedetectMetrics {
            cluster: cluster.clone(),
        },
        DispatchContext::new(Initiator::Ui, "test"),
    ));
    assert!(again.is_ok());
    wait(&mut vcx, "the second run to finish", |vcx| {
        let status = status_of(vcx, &cluster);
        !status.is_detecting() && status.checked_at.is_some()
    });
}
