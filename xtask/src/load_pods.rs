//! `cargo xtask load-pods`: the perf fixture behind the smoothness budget (ADR 0013).
//!
//! Creates `--count` pause pods spread round-robin over `--namespaces` namespaces named
//! `<prefix>-0 .. <prefix>-(N-1)` (prefix from `--namespace`, default `oxikube-load`), every pod and
//! namespace labelled `app=oxikube-load`. `--churn` then deletes and recreates about 1 % of the pods
//! every 5 seconds until Ctrl-C. `--cleanup` deletes the namespaces again.
//!
//! By default the pods name a scheduler nobody runs ([`UNSCHEDULED`]), so they stay `Pending` and
//! the cluster's kube-scheduler never sees them: thousands of unschedulable pods in its queue make
//! it minutes late for every other pod on the cluster (#484). `--schedule` hands them to the
//! default scheduler instead, for a run that needs `Running` pods on a cluster of its own.
//!
//! Safety: every kubectl call carries an explicit `--context` (default `kind-oxikube`), and any
//! context that does not start with `kind-` is refused unless `--allow-non-kind` is given.
//!
//! The manifest generator and the context guard are pure functions so they are unit-tested without
//! a cluster; the kubectl plumbing is exercised against kind by hand (see docs/PERFORMANCE.md).

use anyhow::{Context, Result, bail};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Label carried by every namespace and pod this tool creates.
pub const LABEL_APP: &str = "oxikube-load";
/// Pods per `kubectl apply` call; 10 000 documents in one request is slow and can hit size limits.
pub const APPLY_CHUNK: usize = 500;
/// The `schedulerName` the pods carry unless `--schedule`: no scheduler of that name runs.
pub const UNSCHEDULED: &str = "oxikube-load-unscheduled";
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
    /// Let the cluster's scheduler place the pods (they may run). Without it they name a
    /// scheduler nobody runs and stay `Pending`, which keeps a shared cluster's scheduler free.
    #[arg(long)]
    pub schedule: bool,
}

impl Args {
    /// The `schedulerName` the pods carry: `None` for the default scheduler.
    pub fn scheduler(&self) -> Option<&'static str> {
        (!self.schedule).then_some(UNSCHEDULED)
    }
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

/// True for `<prefix>-<digits>`, the only namespaces `--cleanup` may delete. A bare `<prefix>` is
/// never matched: this tool never creates it, so it could only be a namespace someone else owns
/// (`--namespace kube-system` must not delete kube-system).
pub fn is_load_namespace(prefix: &str, name: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// YAML for one namespace document (starts with `---`).
pub fn namespace_doc(prefix: &str, index: usize) -> String {
    let name = namespace_name(prefix, index);
    format!(
        "---\napiVersion: v1\nkind: Namespace\nmetadata:\n  name: {name}\n  labels:\n    app: {LABEL_APP}\n"
    )
}

/// YAML for one pause pod document (starts with `---`); `scheduler` sets its `schedulerName`.
pub fn pod_doc(i: usize, prefix: &str, namespaces: usize, scheduler: Option<&str>) -> String {
    let name = pod_name(i);
    let ns = namespace_name(prefix, namespace_index(i, namespaces));
    let scheduler = scheduler
        .map(|s| format!("  schedulerName: {s}\n"))
        .unwrap_or_default();
    format!(
        "---\napiVersion: v1\nkind: Pod\nmetadata:\n  name: {name}\n  namespace: {ns}\n  labels:\n    app: {LABEL_APP}\nspec:\n{scheduler}  terminationGracePeriodSeconds: 1\n  containers:\n  - name: pause\n    image: registry.k8s.io/pause:3.10\n    resources:\n      requests:\n        cpu: 1m\n        memory: 1Mi\n"
    )
}

/// Multi-document manifest for pod indices in `range`.
pub fn pods_manifest(
    range: std::ops::Range<usize>,
    prefix: &str,
    namespaces: usize,
    scheduler: Option<&str>,
) -> String {
    range
        .map(|i| pod_doc(i, prefix, namespaces, scheduler))
        .collect()
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
///
/// The SIGINT listener is registered before this returns (inside `block_on`, so it is in the
/// runtime's context), so a Ctrl-C right after start cannot hit the default handler.
fn install_ctrlc() -> Result<Arc<AtomicBool>> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build signal runtime")?;
    let mut sigint = rt
        .block_on(async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        })
        .context("register SIGINT handler")?;
    std::thread::Builder::new()
        .name("ctrlc".into())
        .spawn(move || {
            rt.block_on(async move {
                if sigint.recv().await.is_some() {
                    flag.store(true, Ordering::SeqCst);
                    eprintln!("\nstopping (press Ctrl-C again to abort immediately)");
                    if sigint.recv().await.is_some() {
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

/// Run `kubectl --context <context> <args...>`, optionally feeding `stdin`, and return stdout.
///
/// The child runs in its own process group. Ctrl-C in a terminal signals the whole foreground
/// group, so without this an in-flight kubectl would die with the tool and a churn tick could
/// leave deleted pods unrecreated; this way the tool decides when to stop between kubectl calls.
fn kubectl(context: &str, args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut cmd = Command::new("kubectl");
    cmd.arg("--context")
        .arg(context)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().context("spawn kubectl (is it on PATH?)")?;
    // Feed stdin from a thread so a large manifest cannot deadlock against full output pipes.
    let writer = stdin.map(|input| {
        let mut pipe = child.stdin.take().expect("stdin was piped");
        let input = input.to_owned();
        std::thread::spawn(move || pipe.write_all(input.as_bytes()))
    });
    let out = child.wait_with_output().context("wait for kubectl")?;
    if let Some(w) = writer {
        let _ = w.join();
    }
    if !out.status.success() {
        bail!(
            "kubectl {} failed ({}): {}",
            args.first().copied().unwrap_or(""),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn apply(context: &str, manifest: &str) -> Result<()> {
    kubectl(context, &["apply", "-f", "-"], Some(manifest))
        .map(|_| ())
        .context("kubectl apply")
}

pub fn run(args: &Args) -> Result<()> {
    check_context(&args.context, args.allow_non_kind)?;
    if args.namespaces == 0 || args.namespaces > MAX_NAMESPACES {
        bail!("--namespaces must be between 1 and {MAX_NAMESPACES}");
    }
    if args.cleanup {
        return cleanup(&args.context, &args.namespace);
    }
    if args.count == 0 {
        bail!("--count must be at least 1");
    }
    let stop = install_ctrlc()?;
    let scheduler = args.scheduler();
    let Args {
        count,
        namespaces,
        context,
        namespace: prefix,
        ..
    } = args;

    apply(context, &namespaces_manifest(prefix, *namespaces)).context("create namespaces")?;
    println!(
        "context {context}: creating {count} pods over {namespaces} namespaces ({}), {}",
        namespace_name(prefix, 0) + ", ...",
        match scheduler {
            Some(name) => format!("unscheduled (schedulerName {name}: they stay Pending)"),
            None => "placed by the default scheduler".to_owned(),
        }
    );
    let started = Instant::now();
    let mut done = 0;
    while done < *count {
        if stop.load(Ordering::SeqCst) {
            println!("interrupted after {done}/{count} pods; use --cleanup to remove them");
            return Ok(());
        }
        let end = (done + APPLY_CHUNK).min(*count);
        apply(
            context,
            &pods_manifest(done..end, prefix, *namespaces, scheduler),
        )
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
        churn(context, prefix, *namespaces, *count, scheduler, &stop)?;
    }
    Ok(())
}

/// Delete ~1 % of the pods and recreate them, every 5 s, until `stop`.
fn churn(
    context: &str,
    prefix: &str,
    namespaces: usize,
    count: usize,
    scheduler: Option<&str>,
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
            let mut args = vec!["-n", ns.as_str(), "delete", "pod"];
            args.extend(names.iter().map(String::as_str));
            args.extend(["--ignore-not-found", "--timeout=30s"]);
            let _ = kubectl(context, &args, None);
        }
        // Always recreate what was just deleted, even if Ctrl-C arrived meanwhile, so the fleet
        // is whole when we exit.
        let (first, last) = (window.first().copied().unwrap_or(0), window.len());
        let docs: String = window
            .iter()
            .map(|&i| pod_doc(i, prefix, namespaces, scheduler))
            .collect();
        if let Err(e) = apply(context, &docs) {
            eprintln!("churn: recreate failed: {e:#}");
        }
        tick += 1;
        println!("churn tick {tick}: recycled {last} pod(s) starting at load-{first}");
        interruptible_sleep(CHURN_INTERVAL.saturating_sub(tick_start.elapsed()), stop);
    }
    println!("churn stopped after {tick} tick(s)");
    Ok(())
}

fn cleanup(context: &str, prefix: &str) -> Result<()> {
    // Select by label first, then by name pattern: both must match before anything is deleted.
    let selector = format!("app={LABEL_APP}");
    let listing = kubectl(
        context,
        &["get", "namespaces", "-l", selector.as_str(), "-o", "name"],
        None,
    )
    .context("list namespaces")?;
    let targets: Vec<String> = listing
        .lines()
        .filter_map(|l| l.trim().strip_prefix("namespace/"))
        .filter(|n| is_load_namespace(prefix, n))
        .map(str::to_owned)
        .collect();
    if targets.is_empty() {
        println!(
            "no `{prefix}-<n>` namespaces labelled app={LABEL_APP} in {context}; nothing to clean up"
        );
        return Ok(());
    }
    let n = targets.len();
    println!("deleting namespaces in {context}: {}", targets.join(", "));
    let mut args = vec!["delete", "namespace"];
    args.extend(targets.iter().map(String::as_str));
    args.push("--wait=true");
    kubectl(context, &args, None).context("delete namespaces")?;
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
        let doc = pod_doc(7, "oxikube-load", 4, None);
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
            let doc = pod_doc(i, "p", ns, Some(UNSCHEDULED));
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
        let m = pods_manifest(0..1200, "oxikube-load", 4, Some(UNSCHEDULED));
        assert_eq!(m.matches("---\n").count(), 1200);
        let mut names: Vec<&str> = m.lines().filter(|l| l.starts_with("  name: ")).collect();
        assert_eq!(names.len(), 1200);
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 1200);
        assert_eq!(m.matches("    app: oxikube-load\n").count(), 1200);
        assert_eq!(m.matches("cpu: 1m").count(), 1200);
        assert_eq!(
            m.matches("  schedulerName: oxikube-load-unscheduled\n")
                .count(),
            1200
        );
    }

    #[test]
    fn pods_are_unscheduled_unless_asked() {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: Args,
        }
        let parse = |argv: &[&str]| Cli::try_parse_from(argv).unwrap().args;
        assert_eq!(parse(&["load-pods"]).scheduler(), Some(UNSCHEDULED));
        assert_eq!(parse(&["load-pods", "--schedule"]).scheduler(), None);
        let doc = pod_doc(0, "p", 1, Some(UNSCHEDULED));
        assert!(doc.contains(
            "spec:\n  schedulerName: oxikube-load-unscheduled\n  terminationGracePeriodSeconds: 1\n"
        ));
        assert!(!pod_doc(0, "p", 1, None).contains("schedulerName"));
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
        assert!(
            !is_load_namespace("oxikube-load", "oxikube-load"),
            "bare prefix is never created by this tool"
        );
        assert!(
            !is_load_namespace("kube-system", "kube-system"),
            "--namespace kube-system must not make kube-system deletable"
        );
        assert!(!is_load_namespace("oxikube-fixtures", "oxikube-fixtures"));
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
