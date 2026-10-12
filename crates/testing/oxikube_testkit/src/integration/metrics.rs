//! Metrics fixtures of the kind suites (E13-S13): which tier runs, where the pinned stack lives,
//! readiness waits and series assertions.
//!
//! The metrics path is proven in tiers, each skippable on its own so a developer runs only what
//! their cluster has:
//!
//! | Tier | The cluster needs | Runs when |
//! |---|---|---|
//! | [`MetricsTier::MetricsServer`] | `cargo xtask kind-up` (pinned metrics-server) | `OXIKUBE_TEST_CONTEXT` is set, unless `OXIKUBE_TEST_METRICS_SERVER=0` |
//! | [`MetricsTier::PrometheusStack`] | `cargo xtask kind-metrics-stack` (kube-prometheus-stack) | also `OXIKUBE_TEST_PROMETHEUS_STACK=1` |
//! | [`MetricsTier::VictoriaMetrics`] | the same command (VictoriaMetrics under `/vm`) | also `OXIKUBE_TEST_VICTORIA_METRICS=1` |
//!
//! The stack tiers are the nightly's (`metrics-stack` job); the charts and their versions are
//! `fixtures/metrics-stack/charts.json`, which [`stack_versions`] prints into the test output.
//! The suites' waits are only for readiness (a scrape landing, an endpoint answering) and print
//! [`namespace_diagnostics`] when they run out; assertions are never retried.

use std::process::Command;
use std::time::{Duration, Instant};

use oxikube_ports::PromqlSeries;

use super::{IntegrationError, ensure_kind_context, kubectl, test_context};

/// `0` (or `false`, `off`, `no`) skips the metrics-server tier.
const METRICS_SERVER_ENV: &str = "OXIKUBE_TEST_METRICS_SERVER";
/// `1` (or `true`, `on`, `yes`) runs the kube-prometheus-stack tier.
const PROMETHEUS_STACK_ENV: &str = "OXIKUBE_TEST_PROMETHEUS_STACK";
/// `1` (or `true`, `on`, `yes`) runs the VictoriaMetrics manual-endpoint tier.
const VICTORIA_METRICS_ENV: &str = "OXIKUBE_TEST_VICTORIA_METRICS";

/// The pinned charts of the stack tiers (`cargo xtask kind-metrics-stack` installs them).
const STACK_CHARTS: &str = include_str!("../../fixtures/metrics-stack/charts.json");

/// Where kube-prometheus-stack runs: the namespace and Service the detector's catalogue lists
/// first.
pub const KUBE_PROMETHEUS_NAMESPACE: &str = "monitoring";
/// The Prometheus Service of the stack.
pub const KUBE_PROMETHEUS_SERVICE: &str = "kube-prometheus-stack-prometheus";

/// Where VictoriaMetrics runs: a namespace the catalogue does not list, so only a manual
/// endpoint reaches it.
pub const VICTORIA_NAMESPACE: &str = "oxikube-vm";
/// Its Service.
const VICTORIA_SERVICE: &str = "vmsingle-test";
/// Its port.
const VICTORIA_PORT: u16 = 8428;
/// The path prefix it serves under (`-http.pathPrefix`).
pub const VICTORIA_PATH_PREFIX: &str = "/vm";

/// One tier of the metrics suites. See the [module docs](self).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricsTier {
    /// metrics-server only: tables, the live service, the overview's tiles.
    MetricsServer,
    /// kube-prometheus-stack: auto-detection, the detail panels' and the overview's graphs.
    PrometheusStack,
    /// VictoriaMetrics single-node as a manual endpoint with a path prefix.
    VictoriaMetrics,
}

impl MetricsTier {
    /// The variable that turns the tier on or off.
    pub const fn env(self) -> &'static str {
        match self {
            Self::MetricsServer => METRICS_SERVER_ENV,
            Self::PrometheusStack => PROMETHEUS_STACK_ENV,
            Self::VictoriaMetrics => VICTORIA_METRICS_ENV,
        }
    }

    /// Whether the tier runs when its variable is unset: only metrics-server, which `kind-up`
    /// always installs.
    const fn on_by_default(self) -> bool {
        matches!(self, Self::MetricsServer)
    }

    /// Whether the tier runs given the raw value of its variable.
    pub fn enabled_by(self, raw: Option<&str>) -> bool {
        match raw.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") => self.on_by_default(),
            Some("0" | "false" | "off" | "no") => false,
            Some(_) => true,
        }
    }

    /// Whether the tier runs in this process.
    pub fn is_enabled(self) -> bool {
        self.enabled_by(std::env::var(self.env()).ok().as_deref())
    }
}

/// The kind context to run `tier` against, or `None` (after a skip notice) when the cluster or
/// the tier is not there. A stack tier prints the pinned chart versions first, so the run's log
/// records what it ran against. Panics on a context that is not a kind one.
pub fn tier_context(tier: MetricsTier) -> Option<String> {
    let context = test_context()?;
    ensure_kind_context(&context).expect("a kind context");
    if !tier.is_enabled() {
        eprintln!(
            "skipping the {tier:?} metrics tier: {} turns it {}",
            tier.env(),
            if tier.on_by_default() {
                "off"
            } else {
                "on (`cargo xtask kind-metrics-stack` installs what it needs)"
            }
        );
        return None;
    }
    if tier != MetricsTier::MetricsServer {
        eprintln!("metrics stack under test: {}", stack_versions());
    }
    Some(context)
}

/// One chart of [`STACK_CHARTS`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct StackChart {
    /// The Helm release.
    release: String,
    /// The chart.
    chart: String,
    /// Its pinned version.
    version: String,
    /// The namespace it is installed in.
    namespace: String,
}

/// The charts of [`STACK_CHARTS`], in install order.
fn stack_charts() -> Vec<StackChart> {
    let parsed: serde_json::Value =
        serde_json::from_str(STACK_CHARTS).expect("charts.json is JSON");
    let text = |chart: &serde_json::Value, key: &str| {
        chart[key]
            .as_str()
            .unwrap_or_else(|| panic!("charts.json: `{key}` of {chart}"))
            .to_owned()
    };
    parsed["charts"]
        .as_array()
        .expect("charts.json: `charts`")
        .iter()
        .map(|chart| StackChart {
            release: text(chart, "release"),
            chart: text(chart, "chart"),
            version: text(chart, "version"),
            namespace: text(chart, "namespace"),
        })
        .collect()
}

/// `kube-prometheus-stack 92.3.0, victoria-metrics-single 0.48.0`: what the stack tiers pin.
pub fn stack_versions() -> String {
    stack_charts()
        .iter()
        .map(|c| format!("{} {}", c.chart, c.version))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `oxikube-vm/vmsingle-test:8428`: the VictoriaMetrics Service as the `prometheus.path` setting
/// names it.
pub fn victoria_service_path() -> String {
    format!("{VICTORIA_NAMESPACE}/{VICTORIA_SERVICE}:{VICTORIA_PORT}")
}

/// Whether the `metrics.k8s.io` APIService is `Available` (what `kubectl top` needs).
fn metrics_api_available(context: &str) -> bool {
    Command::new("kubectl")
        .args([
            "--context",
            context,
            "get",
            "apiservice",
            "v1beta1.metrics.k8s.io",
        ])
        .args([
            "-o",
            "jsonpath={.status.conditions[?(@.type==\"Available\")].status}",
        ])
        .output()
        .is_ok_and(|out| out.status.success() && out.stdout.trim_ascii() == b"True")
}

/// Waits up to `timeout` for the `metrics.k8s.io` APIService to be `Available`.
///
/// # Errors
///
/// [`IntegrationError::Kubectl`] when it is not, with the APIService's conditions.
pub fn wait_metrics_api(context: &str, timeout: Duration) -> Result<(), IntegrationError> {
    ensure_kind_context(context)?;
    let started = Instant::now();
    while !metrics_api_available(context) {
        if started.elapsed() > timeout {
            let status = kubectl(context, &["get", "apiservice", "v1beta1.metrics.k8s.io"])
                .err()
                .map_or_else(|| "present but not Available".to_owned(), |e| e.to_string());
            return Err(IntegrationError::Kubectl(format!(
                "metrics.k8s.io not Available after {timeout:?} ({status}); run `cargo xtask kind-up`"
            )));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

/// The pods and events of `namespace`, for a failure message: what the stack was doing when a
/// wait ran out. Never fails; reads only status and events (no logs, no object payloads).
pub fn namespace_diagnostics(context: &str, namespace: &str) -> String {
    let run = |args: &[&str]| {
        Command::new("kubectl")
            .args(["--context", context, "-n", namespace])
            .args(args)
            .output()
            .map_or_else(
                |e| format!("(kubectl could not be run: {e})"),
                |out| {
                    String::from_utf8_lossy(if out.status.success() {
                        &out.stdout
                    } else {
                        &out.stderr
                    })
                    .trim()
                    .to_owned()
                },
            )
    };
    format!(
        "pods in {namespace}:\n{}\nevents in {namespace}:\n{}",
        run(&["get", "pods", "-o", "wide"]),
        run(&[
            "get",
            "events",
            "-o",
            "wide",
            "--sort-by=.metadata.creationTimestamp"
        ]),
    )
}

/// Asserts that `values` (one series' samples) hold at least one finite reading and that every
/// finite reading is non-negative: CPU, memory and network are never below zero. Names `what` in
/// the failure.
pub fn assert_samples(what: &str, values: impl IntoIterator<Item = f64>) {
    let values: Vec<f64> = values.into_iter().collect();
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    assert!(
        !finite.is_empty(),
        "{what}: no reading among {} samples",
        values.len()
    );
    assert!(
        finite.iter().all(|v| *v >= 0.0),
        "{what}: a negative reading in {finite:?}"
    );
}

/// Asserts that the answer to `query` has at least one series and that every series holds
/// readings ([`assert_samples`]). The failure message carries the query, so a nightly failure
/// shows which PromQL broke.
pub fn assert_series(what: &str, query: &str, series: &[PromqlSeries]) {
    assert!(!series.is_empty(), "{what}: no series for `{query}`");
    for one in series {
        assert_samples(
            &format!("{what}: `{query}` {:?}", one.labels),
            one.points.iter().map(|p| p.value),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use jiff::Timestamp;
    use oxikube_ports::PromqlValue;

    use super::*;

    #[test]
    fn metrics_server_runs_unless_turned_off_and_the_stacks_only_when_turned_on() {
        use MetricsTier::*;
        assert!(MetricsServer.enabled_by(None));
        assert!(MetricsServer.enabled_by(Some(" ")));
        assert!(!MetricsServer.enabled_by(Some("0")));
        assert!(!MetricsServer.enabled_by(Some("Off")));
        for tier in [PrometheusStack, VictoriaMetrics] {
            assert!(!tier.enabled_by(None), "{tier:?}");
            assert!(!tier.enabled_by(Some("false")), "{tier:?}");
            assert!(tier.enabled_by(Some("1")), "{tier:?}");
            assert!(tier.enabled_by(Some("yes")), "{tier:?}");
        }
        assert_ne!(PrometheusStack.env(), VictoriaMetrics.env());
    }

    #[test]
    fn the_pinned_charts_are_where_the_constants_say() {
        let charts = stack_charts();
        let stack = charts
            .iter()
            .find(|c| c.chart == "kube-prometheus-stack")
            .expect("kube-prometheus-stack is pinned");
        assert_eq!(stack.namespace, KUBE_PROMETHEUS_NAMESPACE);
        assert_eq!(
            format!("{}-prometheus", stack.release),
            KUBE_PROMETHEUS_SERVICE
        );
        let vm = charts
            .iter()
            .find(|c| c.chart == "victoria-metrics-single")
            .expect("VictoriaMetrics is pinned");
        assert_eq!(vm.namespace, VICTORIA_NAMESPACE);
        assert_eq!(vm.release, VICTORIA_SERVICE);
        assert_eq!(victoria_service_path(), "oxikube-vm/vmsingle-test:8428");
        let versions = stack_versions();
        assert!(versions.contains(&format!("kube-prometheus-stack {}", stack.version)));
        assert!(versions.contains(&format!("victoria-metrics-single {}", vm.version)));
    }

    fn series(values: &[f64]) -> PromqlSeries {
        PromqlSeries {
            labels: BTreeMap::from([("pod".to_owned(), "a".to_owned())]),
            points: values
                .iter()
                .map(|v| PromqlValue {
                    ts: Timestamp::UNIX_EPOCH,
                    value: *v,
                })
                .collect(),
        }
    }

    #[test]
    fn series_with_readings_pass() {
        assert_series("cpu", "up", &[series(&[f64::NAN, 0.0, 1.5])]);
        assert_samples("memory", [3.0]);
    }

    #[test]
    #[should_panic(expected = "no series for `up`")]
    fn no_series_fails_with_the_query() {
        assert_series("cpu", "up", &[]);
    }

    #[test]
    #[should_panic(expected = "no reading")]
    fn a_series_of_gaps_fails() {
        assert_series("cpu", "up", &[series(&[f64::NAN])]);
    }

    #[test]
    #[should_panic(expected = "negative reading")]
    fn a_negative_reading_fails() {
        assert_samples("cpu", [1.0, -0.5]);
    }
}
