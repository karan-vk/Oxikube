//! Throughput of the feed pipeline (watch events in, batches out) at 10k-pod scale, against
//! the scripted server on a real clock. Ignored by default; run it for the PR's perf numbers:
//!
//! ```text
//! cargo test -p oxikube_kube --release --lib feed::tests::perf -- --ignored --nocapture
//! ```

use std::time::Instant;

use oxikube_domain::session::WatchScope;
use oxikube_ports::WatchOptions;

use super::server::*;
use super::*;

const PODS_IN_LIST: usize = 10_000;
const EVENTS: usize = 50_000;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "perf measurement; run with --release --ignored --nocapture"]
async fn ten_thousand_pods_and_fifty_thousand_events() {
    let server = FeedServer::new(31);
    let items = (0..PODS_IN_LIST)
        .map(|p| pod(&format!("p{p}"), &format!("u{p}"), &(p + 1).to_string()))
        .collect();
    server.list(PODS, pod_list(items, &PODS_IN_LIST.to_string()));
    let events = (0..EVENTS)
        .map(|e| {
            let p = e % PODS_IN_LIST;
            let rv = (PODS_IN_LIST + 1 + e).to_string();
            event("MODIFIED", pod(&format!("p{p}"), &format!("u{p}"), &rv))
        })
        .collect();
    server.watch(PODS, Reply::Events(events));

    let started = Instant::now();
    let mut feed = server
        .resources(config())
        .reflector_feed(
            &pod_gvk(),
            &WatchScope::Namespaces(vec!["default".into()]),
            &WatchOptions::default(),
        )
        .await
        .unwrap();
    let mut folded = Folded::default();
    folded.apply(next_batch(&mut feed).await);
    let warm = started.elapsed();
    assert_eq!(folded.0.len(), PODS_IN_LIST);

    let streaming = Instant::now();
    let last_rv = (PODS_IN_LIST + EVENTS).to_string();
    let (mut batches, mut deltas) = (0usize, 0usize);
    while folded.0.values().all(|rv| *rv != last_rv) {
        let batch = next_batch(&mut feed).await;
        batches += 1;
        deltas += batch.len();
        folded.apply(batch);
    }
    let elapsed = streaming.elapsed();
    let per_sec = EVENTS as f64 / elapsed.as_secs_f64();
    eprintln!(
        "feed perf: initial list of {PODS_IN_LIST} pods to first batch {warm:?}; \
         {EVENTS} events in {elapsed:?} ({per_sec:.0} events/s in); \
         {batches} batches out, {deltas} deltas, mean batch {:.0} deltas",
        deltas as f64 / batches.max(1) as f64,
    );
}

/// A pod as a typical cluster serves it: labels, an owner, a spec with env and probes, and a
/// status with conditions and container statuses (about 2.5 KB of JSON).
fn realistic_pod(p: usize) -> serde_json::Value {
    let mut object = meta_pod(&format!("p{p}"), &format!("u{p}"), &(p + 1).to_string());
    let env: Vec<_> = (0..8)
        .map(|i| serde_json::json!({"name": format!("ENV_{i}"), "value": "some-value"}))
        .collect();
    let conditions: Vec<_> = ["Initialized", "Ready", "ContainersReady", "PodScheduled"]
        .iter()
        .map(|t| serde_json::json!({"type": t, "status": "True", "lastTransitionTime": "2026-01-01T00:00:00Z"}))
        .collect();
    object["spec"] = serde_json::json!({
        "nodeName": "node-1", "serviceAccountName": "default", "restartPolicy": "Always",
        "containers": [{
            "name": "app", "image": "registry.example.com/team/app:1.2.3",
            "env": env,
            "resources": {"limits": {"cpu": "500m", "memory": "256Mi"}, "requests": {"cpu": "100m", "memory": "64Mi"}},
            "livenessProbe": {"httpGet": {"path": "/healthz", "port": 8080}, "periodSeconds": 10},
            "volumeMounts": [{"name": "kube-api-access", "mountPath": "/var/run/secrets/kubernetes.io/serviceaccount"}],
        }],
        "volumes": [{"name": "kube-api-access", "projected": {"sources": []}}],
    });
    object["status"] = serde_json::json!({
        "phase": "Running", "podIP": "10.0.0.1", "hostIP": "192.168.0.2", "startTime": "2026-01-01T00:00:00Z",
        "conditions": conditions,
        "containerStatuses": [{"name": "app", "ready": true, "restartCount": 0, "image": "registry.example.com/team/app:1.2.3",
            "containerID": "containerd://0123456789abcdef0123456789abcdef", "state": {"running": {"startedAt": "2026-01-01T00:00:00Z"}}}],
    });
    object
}

/// Resident set size of this process in KiB (`ps`), 0 when unavailable.
fn rss_kib() -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

/// Opens a feed over 10 000 pods and reports the bytes the list response carries, the JSON the
/// consumer then holds and the resident-set growth. Run each variant in its own process so
/// one does not leave the allocator's pages to the other.
async fn memory_of_ten_thousand_pods(metadata: bool) {
    let server = FeedServer::new(31);
    let items: Vec<_> = (0..PODS_IN_LIST)
        .map(|p| {
            if metadata {
                typed_pod(meta_pod(
                    &format!("p{p}"),
                    &format!("u{p}"),
                    &(p + 1).to_string(),
                ))
            } else {
                realistic_pod(p)
            }
        })
        .collect();
    let list = pod_list(items, &PODS_IN_LIST.to_string());
    let Reply::Json(_, body) = &list else {
        unreachable!()
    };
    let wire_bytes = body.to_string().len();
    server.list(PODS, list);
    let resources = server.resources(config());
    let options = if metadata {
        WatchOptions::default().metadata_only()
    } else {
        WatchOptions::default()
    };

    let before = rss_kib();
    let started = Instant::now();
    let mut feed = resources
        .reflector_feed(
            &pod_gvk(),
            &WatchScope::Namespaces(vec!["default".into()]),
            &options,
        )
        .await
        .unwrap();
    let first = next_batch(&mut feed).await;
    let warm = started.elapsed();
    let Delta::Restarted(all) = &first.deltas[0] else {
        panic!("the first delta is the list");
    };
    assert_eq!(all.len(), PODS_IN_LIST);
    assert_eq!(all[0].is_partial(), metadata);
    let held: usize = all.iter().map(|r| r.json.to_string().len()).sum();
    let grown = rss_kib().saturating_sub(before);
    eprintln!(
        "{} feed, {PODS_IN_LIST} pods: list response {:.1} MiB; consumer JSON {:.1} MiB; \
         RSS +{:.1} MiB (batch + store, scripted server included); first batch in {warm:?}",
        if metadata { "metadata" } else { "full" },
        wire_bytes as f64 / 1_048_576.0,
        held as f64 / 1_048_576.0,
        grown as f64 / 1024.0,
    );
}

/// `cargo test -p oxikube_kube --release --lib feed::tests::perf::memory_full -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "perf measurement; run with --release --ignored --nocapture"]
async fn memory_full() {
    memory_of_ten_thousand_pods(false).await;
}

/// `cargo test -p oxikube_kube --release --lib feed::tests::perf::memory_metadata -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "perf measurement; run with --release --ignored --nocapture"]
async fn memory_metadata() {
    memory_of_ten_thousand_pods(true).await;
}
