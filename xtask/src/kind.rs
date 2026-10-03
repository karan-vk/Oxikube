//! Local kind cluster helpers for integration tests and perf fixtures.
//!
//! `kind-up` creates a cluster, installs metrics-server (insecure TLS for kind),
//! and applies `crates/testing/oxikube_testkit/fixtures/cluster/` (sample CRD with
//! printer columns, a few workloads). `load-pods` seeds pause pods for E07 perf work.
//! Requires `kind` and `kubectl` on PATH. Full implementation: E01-S09 / E01-S10.

use anyhow::{Context, Result};
use xshell::{Shell, cmd};

const METRICS_SERVER: &str =
    "https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml";

pub fn up(name: &str) -> Result<()> {
    let sh = Shell::new()?;
    let exists = cmd!(sh, "kind get clusters").read().unwrap_or_default();
    if !exists.lines().any(|l| l.trim() == name) {
        cmd!(sh, "kind create cluster --name {name} --wait 120s")
            .run()
            .context("kind create cluster")?;
    }
    let ctx = format!("kind-{name}");
    cmd!(sh, "kubectl --context {ctx} apply -f {METRICS_SERVER}").run()?;
    let patch = r#"[{"op":"add","path":"/spec/template/spec/containers/0/args/-","value":"--kubelet-insecure-tls"}]"#;
    cmd!(sh, "kubectl --context {ctx} -n kube-system patch deployment metrics-server --type=json -p {patch}").run()?;
    let fixtures = "crates/testing/oxikube_testkit/fixtures/cluster";
    let has_manifests = std::fs::read_dir(fixtures)
        .map(|d| {
            d.flatten().any(|e| {
                matches!(
                    e.path().extension().and_then(|x| x.to_str()),
                    Some("yaml" | "yml" | "json")
                )
            })
        })
        .unwrap_or(false);
    if has_manifests {
        cmd!(sh, "kubectl --context {ctx} apply -R -f {fixtures}").run()?;
    } else {
        println!("no fixture manifests in {fixtures} yet (E01-S09)");
    }
    println!("kind cluster `{name}` ready (context {ctx})");
    Ok(())
}

pub fn down(name: &str) -> Result<()> {
    let sh = Shell::new()?;
    cmd!(sh, "kind delete cluster --name {name}").run()?;
    Ok(())
}

pub fn load_pods(
    count: usize,
    churn: bool,
    namespace: &str,
    context: &str,
    allow_non_kind: bool,
) -> Result<()> {
    if !context.starts_with("kind-") && !allow_non_kind {
        anyhow::bail!(
            "refusing to load pods into non-kind context `{context}`; pass --allow-non-kind if you really mean it"
        );
    }
    let sh = Shell::new()?;
    let _ = cmd!(
        sh,
        "kubectl --context {context} create namespace {namespace}"
    )
    .ignore_status()
    .run();
    let manifest = (0..count)
        .map(|i| {
            format!(
                "apiVersion: v1\nkind: Pod\nmetadata:\n  name: load-{i}\n  namespace: {namespace}\n  labels:\n    app: oxikube-load\nspec:\n  containers:\n  - name: pause\n    image: registry.k8s.io/pause:3.10\n    resources:\n      requests: {{cpu: 1m, memory: 1Mi}}\n---\n"
            )
        })
        .collect::<String>();
    cmd!(sh, "kubectl --context {context} apply -f -")
        .stdin(&manifest)
        .run()?;
    println!("created {count} pods in {namespace}");
    if churn {
        println!("churning: deleting/recreating 1% every 5s (ctrl-c to stop)");
        let step = (count / 100).max(1);
        let mut i = 0usize;
        loop {
            for j in 0..step {
                let n = (i + j) % count;
                let pod = format!("load-{n}");
                let _ = cmd!(
                    sh,
                    "kubectl --context {context} -n {namespace} delete pod {pod} --wait=false"
                )
                .ignore_status()
                .quiet()
                .run();
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
            let chunk = (0..step)
                .map(|j| {
                    let n = (i + j) % count;
                    format!(
                        "apiVersion: v1\nkind: Pod\nmetadata:\n  name: load-{n}\n  namespace: {namespace}\n  labels:\n    app: oxikube-load\nspec:\n  containers:\n  - name: pause\n    image: registry.k8s.io/pause:3.10\n---\n"
                    )
                })
                .collect::<String>();
            let _ = cmd!(sh, "kubectl --context {context} apply -f -")
                .stdin(&chunk)
                .quiet()
                .run();
            i = (i + step) % count;
        }
    }
    Ok(())
}
