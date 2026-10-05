//! Kind integration for E04-S08: a followed stream survives a container restart (and a pod
//! deletion) with no duplicated line and no gap. Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::DeleteParams;
use oxikube_ports::{LogOptions, LogPort};
use oxikube_testkit::integration::TestNamespace;

use common::logs::{Script, create, logs, numbers, read_lines, script_pod, wait_started};
use common::{DEADLINE, wait_until};

/// Writes `seq 0, 1, 2, ...` at 20 lines/s, keeping the next number in `/data` so a restarted
/// instance carries on where the last one stopped, and crashes once, at `crash_at`.
fn counter(crash_at: u32) -> String {
    format!(
        "i=$(cat /data/n 2>/dev/null || echo 0); \
         while true; do \
           echo \"seq $i\"; i=$((i+1)); echo $i > /data/n; \
           if [ $i -eq {crash_at} ] && [ ! -f /data/crashed ]; then touch /data/crashed; exit 1; fi; \
           sleep 0.05; \
         done"
    )
}

#[tokio::test]
async fn a_container_crash_mid_stream_leaves_no_duplicate_and_no_gap() {
    const TOTAL: u64 = 120;
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let logs = logs(&client);
    create(
        &client,
        ns.name(),
        &script_pod(
            "flaky",
            &[],
            "Always",
            &[Script {
                name: "main",
                script: &counter(40),
            }],
        ),
    )
    .await;
    wait_started(&client, ns.name(), "flaky").await;

    let mut stream = logs
        .stream_logs(ns.name(), "flaky", &LogOptions::follow().tail_lines(1000))
        .await
        .expect("stream");
    // The crash comes after 40 lines; the kubelet restarts the container after its backoff
    // (about 10 s), so this read spans the restart.
    let lines = read_lines(&mut stream, TOTAL as usize, Duration::from_secs(120)).await;

    assert_eq!(
        numbers(&lines, "seq"),
        (0..TOTAL).collect::<Vec<_>>(),
        "every number once, in order: no duplicate, no gap"
    );
    let pods = Api::<Pod>::namespaced(client.as_ref().clone(), ns.name());
    let restarts = pods
        .get("flaky")
        .await
        .expect("pod")
        .status
        .and_then(|s| s.container_statuses)
        .and_then(|s| s.first().map(|c| c.restart_count))
        .unwrap_or(0);
    assert!(restarts >= 1, "the container really restarted ({restarts})");
}

#[tokio::test]
async fn the_previous_instance_is_readable_after_a_crash() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let logs = logs(&client);
    create(
        &client,
        ns.name(),
        &script_pod(
            "crashy",
            &[],
            "Always",
            &[Script {
                name: "main",
                script: &counter(10),
            }],
        ),
    )
    .await;
    wait_started(&client, ns.name(), "crashy").await;
    let pods = Api::<Pod>::namespaced(client.as_ref().clone(), ns.name());
    wait_until("one restart", Duration::from_secs(90), || {
        let pods = pods.clone();
        async move {
            let pod = pods.get("crashy").await.ok()?;
            let restarts = pod.status?.container_statuses?.first()?.restart_count;
            (restarts >= 1).then_some(())
        }
    })
    .await;

    let previous = logs
        .stream_logs(
            ns.name(),
            "crashy",
            &LogOptions::default().container("main").previous(),
        )
        .await
        .expect("previous");
    let lines: Vec<_> = previous.map(|l| l.expect("line")).collect().await;

    assert_eq!(numbers(&lines, "seq"), (0..10).collect::<Vec<_>>());
}

#[tokio::test]
async fn deleting_the_pod_ends_the_followed_stream_without_an_error() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let logs = logs(&client);
    create(
        &client,
        ns.name(),
        &script_pod(
            "doomed",
            &[],
            "Always",
            &[Script {
                name: "main",
                script: &counter(1_000_000),
            }],
        ),
    )
    .await;
    wait_started(&client, ns.name(), "doomed").await;
    let mut stream = logs
        .stream_logs(ns.name(), "doomed", &LogOptions::follow())
        .await
        .expect("stream");
    read_lines(&mut stream, 10, DEADLINE).await;

    Api::<Pod>::namespaced(client.as_ref().clone(), ns.name())
        .delete("doomed", &DeleteParams::default())
        .await
        .expect("delete pod");

    let rest = tokio::time::timeout(Duration::from_secs(60), async {
        let mut errors = 0;
        while let Some(item) = stream.next().await {
            errors += usize::from(item.is_err());
        }
        errors
    })
    .await
    .expect("the stream ends once the pod is gone");
    assert_eq!(rest, 0, "a deleted pod is the end of the log, not an error");
}
