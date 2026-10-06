//! Local kind cluster helpers for integration tests and perf fixtures.
//!
//! `kind-up` creates a cluster (if missing), pre-pulls every test image into its nodes
//! (`kind_images.rs`, list in `fixtures/test-images.txt`), installs a pinned metrics-server (insecure
//! kubelet TLS for kind), waits for it, and applies the fixtures under
//! `crates/testing/oxikube_testkit/fixtures/` (sample CRD with printer columns, workloads in
//! several states). It is idempotent and every kubectl call carries `--context kind-<name>`
//! so it can never touch another cluster. (`load-pods` lives in `load_pods.rs`.)
//! Requires `kind` and `kubectl` on PATH.

use crate::kind_images;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use xshell::{Shell, cmd};

/// `crates/testing/oxikube_testkit/fixtures`, resolved from this crate so it works from any cwd.
pub(crate) fn fixtures_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("crates/testing/oxikube_testkit/fixtures");
    dir.canonicalize().unwrap_or(dir)
}

/// Subdirectories of `cluster/` in name order (`00-crds`, `10-namespaces`, ...).
fn fixture_stages(cluster_dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(cluster_dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

/// A stage that defines CRDs must be Established before the next stage creates CRs.
fn is_crd_stage(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with("crds"))
}

pub fn up(name: &str) -> Result<()> {
    let sh = Shell::new()?;
    let exists = cmd!(sh, "kind get clusters").read().unwrap_or_default();
    if !exists.lines().any(|l| l.trim() == name) {
        cmd!(sh, "kind create cluster --name {name} --wait 120s")
            .run()
            .context("kind create cluster")?;
    }
    let ctx = format!("kind-{name}");
    let fixtures = fixtures_dir();

    // Every image the suites and fixtures run is pulled into the nodes first, so nothing below
    // (nor any test later) waits on a registry. Idempotent: present images are skipped.
    kind_images::preload(name).context("pre-pull the test images")?;

    // metrics-server: pinned release via kustomize (adds --kubelet-insecure-tls), so re-running
    // is a no-op instead of re-appending the flag.
    let ms = fixtures.join("metrics-server");
    cmd!(sh, "kubectl --context {ctx} apply -k {ms}")
        .run()
        .context("apply metrics-server")?;
    cmd!(
        sh,
        "kubectl --context {ctx} -n kube-system rollout status deployment/metrics-server --timeout=180s"
    )
    .run()
    .context("wait for metrics-server rollout")?;

    // Fixtures, stage by stage; CRDs are established before any CR is applied.
    for stage in fixture_stages(&fixtures.join("cluster")) {
        cmd!(sh, "kubectl --context {ctx} apply -R -f {stage}")
            .run()
            .with_context(|| format!("apply fixtures {}", stage.display()))?;
        if is_crd_stage(&stage) {
            cmd!(
                sh,
                "kubectl --context {ctx} wait --for=condition=established --timeout=60s -R -f {stage}"
            )
            .run()
            .context("wait for CRDs to be established")?;
        }
    }
    cmd!(
        sh,
        "kubectl --context {ctx} -n oxikube-fixtures wait --for=condition=available --timeout=120s deployment/fixtures-web"
    )
    .run()
    .context("wait for fixture deployment")?;

    println!("kind cluster `{name}` ready (context {ctx})");
    Ok(())
}

pub fn down(name: &str) -> Result<()> {
    let sh = Shell::new()?;
    cmd!(sh, "kind delete cluster --name {name}").run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crd_stage_detection() {
        assert!(is_crd_stage(Path::new("fixtures/cluster/00-crds")));
        assert!(!is_crd_stage(Path::new("fixtures/cluster/20-workloads")));
    }

    #[test]
    fn fixture_stages_are_ordered_and_crds_come_first() {
        let stages = fixture_stages(&fixtures_dir().join("cluster"));
        let names: Vec<_> = stages
            .iter()
            .filter_map(|p| p.file_name()?.to_str().map(str::to_owned))
            .collect();
        assert!(names.len() >= 3, "{names:?}");
        assert!(is_crd_stage(&stages[0]), "{names:?}");
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn metrics_server_is_pinned() {
        let k = std::fs::read_to_string(fixtures_dir().join("metrics-server/kustomization.yaml"))
            .unwrap();
        assert!(!k.contains("/latest/"), "metrics-server must be pinned");
        assert!(k.contains("--kubelet-insecure-tls"));
    }
}
