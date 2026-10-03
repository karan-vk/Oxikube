//! `cargo xtask load-pods`: the perf fixture behind the smoothness budget (ADR 0013).
//!
//! Creates `--count` pause pods spread round-robin over `--namespaces` namespaces named
//! `<prefix>-0 .. <prefix>-(N-1)` (prefix from `--namespace`, default `oxikube-load`), every pod and
//! namespace labelled `app=oxikube-load`. `--churn` then deletes and recreates about 1 % of the pods
//! every 5 seconds until Ctrl-C. `--cleanup` deletes the namespaces again.
//!
//! Safety: every kubectl call carries an explicit `--context` (default `kind-oxikube`), and any
//! context that does not start with `kind-` is refused unless `--allow-non-kind` is given.
//!
//! The manifest generator and the context guard are pure functions so they are unit-tested without
//! a cluster; the kubectl plumbing is exercised against kind by hand (see docs/PERFORMANCE.md).

use anyhow::{Context, Result, bail};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use xshell::{Shell, cmd};

/// Label carried by every namespace and pod this tool creates.
pub const LABEL_APP: &str = "oxikube-load";
/// Pods per `kubectl apply` call; 10 000 documents in one request is slow and can hit size limits.
pub const APPLY_CHUNK: usize = 500;
/// Seconds between churn ticks.
const CHURN_INTERVAL: Duration = Duration::from_secs(5);
const MAX_NAMESPACES: usize = 1000;

#[derive(clap::Args, Debug, Clone)]
pub struct Args {
    /// Number of pause pods to create.
    #[arg(long, default_value_t = 1000)]
    pub count: usize,
    /// Keep deleting and recreating ~1 % of the pods every 5 s until Ctrl-C.
    #[arg(long)]
    pub churn: bool,
    /// Namespace name prefix; namespaces are `<prefix>-0 .. <prefix>-(N-1)`.
    #[arg(long, default_value = "oxikube-load")]
    pub namespace: String,
    /// Number of namespaces to spread the pods over (round-robin).
    #[arg(long, default_value_t = 4)]
    pub namespaces: usize,
    /// kubectl context to target. Must be a kind context (`kind-*`) unless `--allow-non-kind` is
    /// passed, so this never runs against a real cluster by accident.
    #[arg(long, default_value = "kind-oxikube")]
    pub context: String,
    /// Allow a context that does not start with `kind-`. Creates and deletes many pods; be sure.
    #[arg(long)]
    pub allow_non_kind: bool,
    /// Delete the load namespaces (and every pod in them) and exit.
    #[arg(long)]
    pub cleanup: bool,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

/// Refuse anything that is not a local kind context unless explicitly overridden.
pub fn check_context(context: &str, allow_non_kind: bool) -> Result<()> {
    if context.starts_with("kind-") || allow_non_kind {
        return Ok(());
    }
    bail!(
        "refusing to touch non-kind context `{context}`; load-pods creates and deletes thousands of \
         pods. Use a `kind-*` context (default `kind-oxikube`) or pass --allow-non-kind if you really \
         mean it"
    )
}

/// Name of namespace `index` for a prefix.
pub fn namespace_name(prefix: &str, index: usize) -> String {
    format!("{prefix}-{index}")
}

/// Index of the namespace pod `i` lives in (round-robin).
pub fn namespace_index(i: usize, namespaces: usize) -> usize {
    i % namespaces
}

pub fn pod_name(i: usize) -> String {
    format!("load-{i}")
}

/// True for `<prefix>` or `<prefix>-<digits>`, the only namespaces `--cleanup` may delete.
pub fn is_load_namespace(prefix: &str, name: &str) -> bool {
    match name.strip_prefix(prefix) {
        Some("") => true,
        Some(rest) => rest
            .strip_prefix('-')
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())),
        None => false,
    }
}

/// YAML for one namespace document (starts with `---`).
pub fn namespace_doc(prefix: &str, index: usize) -> String {
    let name = namespace_name(prefix, index);
    format!(
        "---\napiVersion: v1\nkind: Namespace\nmetadata:\n  name: {name}\n  labels:\n    app: {LABEL_APP}\n"
    )
}

/// YAML for one pause pod document (starts with `---`).
pub fn pod_doc(i: usize, prefix: &str, namespaces: usize) -> String {
    let name = pod_name(i);
    let ns = namespace_name(prefix, namespace_index(i, namespaces));
    format!(
        "---\napiVersion: v1\nkind: Pod\nmetadata:\n  name: {name}\n  namespace: {ns}\n  labels:\n    app: {LABEL_APP}\nspec:\n  terminationGracePeriodSeconds: 1\n  containers:\n  - name: pause\n    image: registry.k8s.io/pause:3.10\n    resources:\n      requests:\n        cpu: 1m\n        memory: 1Mi\n"
    )
}

/// Multi-document manifest for pod indices in `range`.
pub fn pods_manifest(range: std::ops::Range<usize>, prefix: &str, namespaces: usize) -> String {
    range.map(|i| pod_doc(i, prefix, namespaces)).collect()
}

/// Multi-document manifest for all namespaces.
pub fn namespaces_manifest(prefix: &str, namespaces: usize) -> String {
    (0..namespaces).map(|n| namespace_doc(prefix, n)).collect()
}

/// Pod indices to churn on tick `tick`: a sliding window of `step` indices over `0..count`, so
/// every pod is cycled in turn and the window wraps.
pub fn churn_window(tick: usize, count: usize, step: usize) -> Vec<usize> {
    (0..step).map(|j| (tick * step + j) % count).collect()
}

/// Pods to cycle per tick: about 1 % of the fleet, at least one.
pub fn churn_step(count: usize) -> usize {
    (count / 100).max(1)
}

// ---------------------------------------------------------------------------
// Ctrl-C
// ---------------------------------------------------------------------------

/// Flag flipped by the first Ctrl-C; a second Ctrl-C exits immediately with status 130.
fn install_ctrlc() -> Result<Arc<AtomicBool>> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build signal runtime")?;
    std::thread::Builder::new()
        .name("ctrlc".into())
        .spawn(move || {
            rt.block_on(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    flag.store(true, Ordering::SeqCst);
                    eprintln!("\nstopping (press Ctrl-C again to abort immediately)");
                    if tokio::signal::ctrl_c().await.is_ok() {
                        std::process::exit(130);
                    }
                }
            });
        })
        .context("spawn signal thread")?;
    Ok(stop)
}

/// Sleep up to `total`, waking early when `stop` is set.
fn interruptible_sleep(total: Duration, stop: &AtomicBool) {
    let deadline = Instant::now() + total;
    while !stop.load(Ordering::SeqCst) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        std::thread::sleep(left.min(Duration::from_millis(100)));
    }
}

// ---------------------------------------------------------------------------
// kubectl plumbing
// ---------------------------------------------------------------------------

fn apply(sh: &Shell, context: &str, manifest: &str) -> Result<()> {
    cmd!(sh, "kubectl --context {context} apply -f -")
        .stdin(manifest)
        .quiet()
        .read()
        .map(|_| ())
        .context("kubectl apply")
}

pub fn run(args: &Args) -> Result<()> {
    check_context(&args.context, args.allow_non_kind)?;
    if args.namespaces == 0 || args.namespaces > MAX_NAMESPACES {
        bail!("--namespaces must be between 1 and {MAX_NAMESPACES}");
    }
    let sh = Shell::new()?;
    if args.cleanup {
        return cleanup(&sh, &args.context, &args.namespace);
    }
    if args.count == 0 {
        bail!("--count must be at least 1");
    }
    let stop = install_ctrlc()?;
    let Args {
        count,
        namespaces,
        context,
        namespace: prefix,
        ..
    } = args;

    apply(&sh, context, &namespaces_manifest(prefix, *namespaces)).context("create namespaces")?;
    println!(
        "context {context}: creating {count} pods over {namespaces} namespaces ({})",
        namespace_name(prefix, 0) + ", ..."
    );
    let started = Instant::now();
    let mut done = 0;
    while done < *count {
        if stop.load(Ordering::SeqCst) {
            println!("interrupted after {done}/{count} pods; use --cleanup to remove them");
            return Ok(());
        }
        let end = (done + APPLY_CHUNK).min(*count);
        apply(&sh, context, &pods_manifest(done..end, prefix, *namespaces))
            .with_context(|| format!("apply pods {done}..{end}"))?;
        done = end;
        println!(
            "applied {done}/{count} pods ({:.0}%, {:.1}s)",
            done as f64 * 100.0 / *count as f64,
            started.elapsed().as_secs_f64()
        );
    }
    println!("created {count} pods in {namespaces} namespaces");

    if args.churn {
        churn(&sh, context, prefix, *namespaces, *count, &stop)?;
    }
    Ok(())
}

/// Delete ~1 % of the pods and recreate them, every 5 s, until `stop`.
fn churn(
    sh: &Shell,
    context: &str,
    prefix: &str,
    namespaces: usize,
    count: usize,
    stop: &AtomicBool,
) -> Result<()> {
    let step = churn_step(count);
    println!(
        "churning {step} pod(s) every {}s across {namespaces} namespaces (Ctrl-C to stop)",
        CHURN_INTERVAL.as_secs()
    );
    let mut tick = 0usize;
    while !stop.load(Ordering::SeqCst) {
        let tick_start = Instant::now();
        let window = churn_window(tick, count, step);
        for ns_idx in 0..namespaces {
            let names: Vec<String> = window
                .iter()
                .filter(|&&i| namespace_index(i, namespaces) == ns_idx)
                .map(|&i| pod_name(i))
                .collect();
            if names.is_empty() {
                continue;
            }
            let ns = namespace_name(prefix, ns_idx);
            // Wait for the old object to disappear, otherwise the re-apply below would patch the
            // terminating pod instead of creating a fresh one. Errors are not fatal for a load tool.
            let _ = cmd!(
                sh,
                "kubectl --context {context} -n {ns} delete pod {names...} --ignore-not-found --timeout=30s"
            )
            .quiet()
            .read();
        }
        // Always recreate what was just deleted, even if Ctrl-C arrived meanwhile, so the fleet
        // is whole when we exit.
        let (first, last) = (window.first().copied().unwrap_or(0), window.len());
        let docs: String = window
            .iter()
            .map(|&i| pod_doc(i, prefix, namespaces))
            .collect();
        if let Err(e) = apply(sh, context, &docs) {
            eprintln!("churn: recreate failed: {e:#}");
        }
        tick += 1;
        println!("churn tick {tick}: recycled {last} pod(s) starting at load-{first}");
        interruptible_sleep(CHURN_INTERVAL.saturating_sub(tick_start.elapsed()), stop);
    }
    println!("churn stopped after {tick} tick(s)");
    Ok(())
}

fn cleanup(sh: &Shell, context: &str, prefix: &str) -> Result<()> {
    let listing = cmd!(sh, "kubectl --context {context} get namespaces -o name")
        .quiet()
        .read()
        .context("list namespaces")?;
    let targets: Vec<String> = listing
        .lines()
        .filter_map(|l| l.trim().strip_prefix("namespace/"))
        .filter(|n| is_load_namespace(prefix, n))
        .map(str::to_owned)
        .collect();
    if targets.is_empty() {
        println!("no `{prefix}*` namespaces in {context}; nothing to clean up");
        return Ok(());
    }
    let n = targets.len();
    println!("deleting namespaces in {context}: {}", targets.join(", "));
    cmd!(
        sh,
        "kubectl --context {context} delete namespace {targets...} --wait=true"
    )
    .quiet()
    .read()
    .context("delete namespaces")?;
    println!("cleaned up {n} namespace(s)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn context_guard() {
        assert!(check_context("kind-oxikube", false).is_ok());
        assert!(check_context("kind-other", false).is_ok());
        let err = check_context("prod-eu", false).unwrap_err().to_string();
        assert!(
            err.contains("prod-eu") && err.contains("--allow-non-kind"),
            "{err}"
        );
        assert!(
            check_context("kind", false).is_err(),
            "needs the `kind-` prefix"
        );
        assert!(check_context("", false).is_err());
        assert!(check_context("prod-eu", true).is_ok(), "override accepted");
    }

    #[test]
    fn pod_manifest_fields() {
        let doc = pod_doc(7, "oxikube-load", 4);
        let expected = "---
apiVersion: v1
kind: Pod
metadata:
  name: load-7
  namespace: oxikube-load-3
  labels:
    app: oxikube-load
spec:
  terminationGracePeriodSeconds: 1
  containers:
  - name: pause
    image: registry.k8s.io/pause:3.10
    resources:
      requests:
        cpu: 1m
        memory: 1Mi
";
        assert_eq!(doc, expected);
    }

    #[test]
    fn round_robin_spread_is_even() {
        let (count, ns) = (1003, 4);
        let mut per: HashMap<String, usize> = HashMap::new();
        for i in 0..count {
            let doc = pod_doc(i, "p", ns);
            let line = doc.lines().find(|l| l.contains("namespace:")).unwrap();
            *per.entry(line.trim().to_owned()).or_default() += 1;
        }
        assert_eq!(per.len(), ns);
        let (min, max) = (per.values().min().unwrap(), per.values().max().unwrap());
        assert!(max - min <= 1, "{per:?}");
        assert_eq!(per["namespace: p-0"], 251);
    }

    #[test]
    fn names_are_unique_and_documents_counted() {
        let m = pods_manifest(0..1200, "oxikube-load", 4);
        assert_eq!(m.matches("---\n").count(), 1200);
        let mut names: Vec<&str> = m.lines().filter(|l| l.starts_with("  name: ")).collect();
        assert_eq!(names.len(), 1200);
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 1200);
        assert_eq!(m.matches("    app: oxikube-load\n").count(), 1200);
        assert_eq!(m.matches("cpu: 1m").count(), 1200);
    }

    #[test]
    fn namespace_manifest_and_names() {
        assert_eq!(namespace_name("oxikube-load", 2), "oxikube-load-2");
        let m = namespaces_manifest("oxikube-load", 3);
        assert_eq!(m.matches("kind: Namespace").count(), 3);
        for n in 0..3 {
            assert!(m.contains(&format!("name: oxikube-load-{n}\n")));
        }
        assert_eq!(m.matches("app: oxikube-load").count(), 3);
    }

    #[test]
    fn cleanup_only_matches_load_namespaces() {
        assert!(is_load_namespace("oxikube-load", "oxikube-load"));
        assert!(is_load_namespace("oxikube-load", "oxikube-load-0"));
        assert!(is_load_namespace("oxikube-load", "oxikube-load-12"));
        assert!(!is_load_namespace("oxikube-load", "oxikube-load-"));
        assert!(!is_load_namespace("oxikube-load", "oxikube-loadtest"));
        assert!(!is_load_namespace("oxikube-load", "oxikube-fixtures"));
        assert!(!is_load_namespace("oxikube-load", "kube-system"));
        assert!(!is_load_namespace("oxikube-load", "oxikube-load-a"));
    }

    #[test]
    fn churn_cycles_one_percent_and_wraps() {
        assert_eq!(churn_step(10_000), 100);
        assert_eq!(churn_step(200), 2);
        assert_eq!(churn_step(5), 1);
        assert_eq!(churn_window(0, 200, 2), vec![0, 1]);
        assert_eq!(churn_window(99, 200, 2), vec![198, 199]);
        assert_eq!(churn_window(100, 200, 2), vec![0, 1]);
        // 100 ticks visit every pod exactly once
        let mut seen: Vec<usize> = (0..100).flat_map(|t| churn_window(t, 200, 2)).collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..200).collect::<Vec<_>>());
    }
}
