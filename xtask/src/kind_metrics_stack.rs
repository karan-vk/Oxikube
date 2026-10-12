//! `cargo xtask kind-metrics-stack`: the tier-2 metrics fixtures of the nightly suite (E13-S13).
//!
//! Installs the charts pinned in `crates/testing/oxikube_testkit/fixtures/metrics-stack/charts.json`
//! (kube-prometheus-stack in `monitoring`, VictoriaMetrics single-node under the path prefix `/vm`
//! in `oxikube-vm`) into a kind TEST cluster with Helm, waits until Prometheus and VictoriaMetrics
//! are ready, and prints the chart and app versions so a nightly log records what it ran against.
//! `--down` uninstalls them again.
//!
//! This is a test fixture: Oxikube itself never installs a metrics stack into a user's cluster.
//! Every `helm` call carries `--kube-context kind-<name>` and every `kubectl` call
//! `--context kind-<name>`, and the cluster must be one `kind get clusters` lists, so it can never
//! touch another cluster. Requires `kind`, `kubectl` and `helm` (3.8 or later) on PATH.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use xshell::{Shell, cmd};

use crate::kind::fixtures_dir;

/// How long the stack may take to become ready: image pulls on a fresh runner plus the operator
/// creating the Prometheus StatefulSet. Generous but finite.
const READY_TIMEOUT: Duration = Duration::from_secs(600);

/// One pinned chart of `charts.json`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct Chart {
    pub release: String,
    pub chart: String,
    pub repo: String,
    pub version: String,
    pub namespace: String,
    pub values: String,
}

#[derive(Deserialize)]
struct Charts {
    charts: Vec<Chart>,
}

fn stack_dir() -> PathBuf {
    fixtures_dir().join("metrics-stack")
}

/// The pinned charts, in install order.
pub(crate) fn charts(dir: &Path) -> Result<Vec<Chart>> {
    let path = dir.join("charts.json");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let parsed: Charts =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(parsed.charts)
}

/// Refuses anything but an existing kind cluster called `name`.
fn ensure_kind_cluster(sh: &Shell, name: &str) -> Result<String> {
    let clusters = cmd!(sh, "kind get clusters")
        .read()
        .context("kind get clusters")?;
    if !clusters.lines().any(|l| l.trim() == name) {
        bail!("no kind cluster `{name}`: run `cargo xtask kind-up --name {name}` first");
    }
    Ok(format!("kind-{name}"))
}

pub fn up(name: &str) -> Result<()> {
    let sh = Shell::new()?;
    let ctx = ensure_kind_cluster(&sh, name)?;
    let dir = stack_dir();
    for chart in charts(&dir)? {
        let Chart {
            release,
            chart: chart_name,
            repo,
            version,
            namespace,
            values,
        } = &chart;
        let values = dir.join(values);
        // `upgrade --install` keeps a re-run idempotent; `--wait` covers what the chart creates
        // itself (the operator, kube-state-metrics, node-exporter, VictoriaMetrics).
        cmd!(
            sh,
            "helm upgrade --install {release} {chart_name} --repo {repo} --version {version}
             --kube-context {ctx} --namespace {namespace} --create-namespace
             --values {values} --wait --timeout 10m"
        )
        .run()
        .with_context(|| format!("install {chart_name} {version}"))?;
    }
    // The operator creates Prometheus' StatefulSet after the chart is installed, so `--wait`
    // cannot cover it: wait for its pod to exist, then to be ready.
    wait_for_pod(
        &sh,
        &ctx,
        "monitoring",
        "app.kubernetes.io/name=prometheus",
        READY_TIMEOUT,
    )?;
    wait_for_pod(
        &sh,
        &ctx,
        "oxikube-vm",
        "app.kubernetes.io/instance=vmsingle-test",
        READY_TIMEOUT,
    )?;
    print_versions(&sh, &ctx)?;
    println!("metrics stack ready on kind cluster `{name}` (context {ctx})");
    Ok(())
}

pub fn down(name: &str) -> Result<()> {
    let sh = Shell::new()?;
    let ctx = ensure_kind_cluster(&sh, name)?;
    for chart in charts(&stack_dir())?.iter().rev() {
        let (release, namespace) = (&chart.release, &chart.namespace);
        cmd!(
            sh,
            "helm uninstall {release} --kube-context {ctx} --namespace {namespace} --ignore-not-found --wait"
        )
        .run()
        .with_context(|| format!("uninstall {release}"))?;
        cmd!(
            sh,
            "kubectl --context {ctx} delete namespace {namespace} --ignore-not-found"
        )
        .run()
        .with_context(|| format!("delete namespace {namespace}"))?;
    }
    Ok(())
}

/// Polls until a pod matching `selector` exists in `namespace` (readiness only, no assertion is
/// retried), then waits for it to be Ready.
fn wait_for_pod(
    sh: &Shell,
    ctx: &str,
    namespace: &str,
    selector: &str,
    timeout: Duration,
) -> Result<()> {
    let started = Instant::now();
    loop {
        let names = cmd!(
            sh,
            "kubectl --context {ctx} -n {namespace} get pods -l {selector} -o name"
        )
        .read()
        .unwrap_or_default();
        if !names.trim().is_empty() {
            break;
        }
        if started.elapsed() > timeout {
            bail!("no pod `{selector}` in `{namespace}` after {timeout:?}");
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let left = timeout.saturating_sub(started.elapsed()).as_secs().max(1);
    let left = format!("--timeout={left}s");
    cmd!(
        sh,
        "kubectl --context {ctx} -n {namespace} wait --for=condition=Ready pod -l {selector} {left}"
    )
    .run()
    .with_context(|| format!("wait for `{selector}` in `{namespace}`"))?;
    Ok(())
}

/// Prints `release chart-version app-version` for every installed release of the stack.
fn print_versions(sh: &Shell, ctx: &str) -> Result<()> {
    let listed = cmd!(
        sh,
        "helm list --kube-context {ctx} --all-namespaces -o json"
    )
    .read()
    .context("helm list")?;
    let releases: Vec<serde_json::Value> = serde_json::from_str(&listed).context("helm list")?;
    for chart in charts(&stack_dir())? {
        let found = releases
            .iter()
            .find(|r| r["name"] == chart.release.as_str());
        let field = |key: &str| {
            found
                .and_then(|r| r[key].as_str())
                .unwrap_or("?")
                .to_owned()
        };
        println!(
            "metrics-stack: {} {} ({}, app {})",
            chart.release,
            field("chart"),
            chart.namespace,
            field("app_version")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_chart_is_pinned_and_has_its_values_file() {
        let dir = stack_dir();
        let charts = charts(&dir).expect("charts.json");
        let names: Vec<_> = charts.iter().map(|c| c.chart.as_str()).collect();
        assert_eq!(names, ["kube-prometheus-stack", "victoria-metrics-single"]);
        for chart in &charts {
            assert!(
                chart.version.split('.').count() == 3
                    && chart
                        .version
                        .chars()
                        .all(|c| c.is_ascii_digit() || c == '.'),
                "{} must pin an exact version, not `{}`",
                chart.chart,
                chart.version
            );
            assert!(chart.repo.starts_with("https://"), "{chart:?}");
            assert!(dir.join(&chart.values).is_file(), "{chart:?}");
        }
    }

    #[test]
    fn the_stack_lands_where_the_detector_looks_and_victoria_metrics_where_it_does_not() {
        let charts = charts(&stack_dir()).expect("charts.json");
        let stack = &charts[0];
        // `<release>-prometheus` in `monitoring`: the catalogue's first kube_prometheus entry.
        assert_eq!(
            (stack.release.as_str(), stack.namespace.as_str()),
            ("kube-prometheus-stack", "monitoring")
        );
        // A namespace the catalogue does not list: only the manual endpoint reaches it.
        let vm = &charts[1];
        assert_eq!(vm.namespace, "oxikube-vm");
        let values =
            std::fs::read_to_string(stack_dir().join(&vm.values)).expect("the values file");
        assert!(values.contains("http.pathPrefix: /vm"), "{values}");
        assert!(
            values.contains("fullnameOverride: vmsingle-test"),
            "{values}"
        );
    }
}
