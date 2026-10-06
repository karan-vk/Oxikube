//! `LogService` (E08-S01) over the real `LogPort` adapter: a busybox pod that echoes a numbered
//! line every 200 ms, read through a session on the Tokio runtime and the wall clock.
//!
//! * a followed session streams the pod's lines in order, with the kubelet's timestamps, and
//!   batches them (far fewer commits than lines);
//! * `tail` and the buffer bound hold against a live stream;
//! * dropping the session cancels the read;
//! * a pod that does not exist is a `Failed(NotFound)` session.
//!
//! The pod lives in the test's own `oxi-test-<rand>` namespace.

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::PostParams;
use oxikube_app::logs::{EndReason, LogConfig, LogRuntime, LogService, LogState, LogTarget};
use oxikube_app::store::Spawner;
use oxikube_domain::ErrorKind;
use oxikube_kube::KubeLogs;
use oxikube_ports::LogOptions;
use oxikube_testkit::images::BUSYBOX;
use oxikube_testkit::integration::TestNamespace;

use crate::clock::TokioClock;
use crate::cluster::Kind;
use crate::eventually;

fn echo_pod(name: &str) -> Pod {
    serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": name, "labels": { "oxikube.test/suite": "logs-service" } },
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": [{
                "name": "main",
                "image": BUSYBOX,
                "command": ["sh", "-c",
                    "i=0; while true; do echo \"line $i\"; i=$((i+1)); sleep 0.2; done"],
            }],
        },
    }))
    .expect("a pod")
}

fn service(buffer_lines: usize) -> LogService {
    service_with(LogConfig {
        buffer_lines,
        ..LogConfig::default()
    })
}

fn service_with(config: LogConfig) -> LogService {
    let spawner: Arc<dyn Spawner> = Arc::new(|task: BoxFuture<'static, ()>| {
        tokio::spawn(task);
    });
    LogService::new(
        LogRuntime {
            spawner,
            clock: Arc::new(TokioClock),
        },
        config,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_followed_session_streams_a_live_pod_and_cancels_on_drop() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    pods.create(&PostParams::default(), &echo_pod("echo"))
        .await
        .expect("create the pod");
    eventually(
        "the pod to run",
        || String::new(),
        || async {
            pods.get("echo")
                .await
                .ok()
                .and_then(|p| p.status)
                .and_then(|s| s.phase)
                .is_some_and(|phase| phase == "Running")
        },
    )
    .await;

    let logs = Arc::new(KubeLogs::new(client.clone()));
    // A flush tick several lines wide (the pod emits five a second), so batching is observable:
    // with the default 32 ms tick every line would be a batch of its own.
    let service = service_with(LogConfig {
        flush_interval: Duration::from_millis(800),
        ..LogConfig::default()
    });
    let session = service.open(
        logs.clone(),
        LogTarget::pod(ns.name(), "echo"),
        LogOptions::follow(),
    );
    let reader = session.reader();
    eventually(
        "ten lines",
        || format!("{:?} with {} lines", reader.state(), reader.len()),
        || async { reader.len() >= 10 },
    )
    .await;

    reader.read(|buffer, state| {
        assert_eq!(*state, LogState::Streaming);
        let seqs: Vec<u64> = buffer.iter().map(|e| e.seq).collect();
        assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "{seqs:?}");
        let first = buffer.get(0).expect("a line");
        assert!(first.text.starts_with("line "), "{}", first.text);
        assert_eq!(&*first.pod, "echo");
        assert_eq!(&*first.container, "main");
        assert!(
            first.ts.as_second() > 1_700_000_000,
            "the kubelet's timestamp"
        );
        assert!(
            buffer
                .iter()
                .zip(buffer.iter().skip(1))
                .all(|(a, b)| a.ts <= b.ts),
            "timestamps are ordered"
        );
        for (i, entry) in buffer.iter().enumerate() {
            let number: usize = entry.text.strip_prefix("line ").unwrap().parse().unwrap();
            assert_eq!(number as u64, entry.seq, "line {i}");
        }
    });
    // Batched: lines arrive five a second and a batch spans 800 ms, so a batch holds ~4 lines. A
    // commit per line (the regression) would make batches == lines.
    assert!(
        reader.batches() * 2 <= reader.len() as u64,
        "{} batches for {} lines",
        reader.batches(),
        reader.len()
    );

    // Cancel on drop: the session ends, and the service lists nothing.
    drop(session);
    assert_eq!(reader.state(), LogState::Ended(EndReason::Cancelled));
    assert!(service.sessions().is_empty());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(reader.state(), LogState::Ended(EndReason::Cancelled));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_buffer_bound_and_tail_hold_against_a_live_stream() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    pods.create(&PostParams::default(), &echo_pod("echo"))
        .await
        .expect("create the pod");
    eventually(
        "the pod to run",
        || String::new(),
        || async {
            pods.get("echo")
                .await
                .ok()
                .and_then(|p| p.status)
                .and_then(|s| s.phase)
                .is_some_and(|phase| phase == "Running")
        },
    )
    .await;
    // Let it write a few lines first, so `tail` has something to cut.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let logs = Arc::new(KubeLogs::new(client.clone()));
    let service = service(oxikube_app::logs::MIN_BUFFER_LINES);
    let session = service.open(
        logs,
        LogTarget::pod(ns.name(), "echo"),
        LogOptions::follow().tail_lines(5),
    );
    eventually(
        "the tail and one new line",
        || format!("{:?} with {} lines", session.state(), session.len()),
        || async { session.len() >= 6 },
    )
    .await;
    session.read(|buffer, _| {
        assert!(buffer.len() <= buffer.capacity());
        let first: usize = buffer
            .get(0)
            .unwrap()
            .text
            .strip_prefix("line ")
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            first >= 5,
            "tail=5 starts after the first lines: line {first}"
        );
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pod_that_does_not_exist_is_a_failed_session() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let service = service(1_000);
    let session = service.open(
        Arc::new(KubeLogs::new(client)),
        LogTarget::pod(ns.name(), "nope"),
        LogOptions::follow(),
    );
    eventually(
        "a failed session",
        || format!("{:?}", session.state()),
        || async { matches!(session.state(), LogState::Failed(_)) },
    )
    .await;
    let LogState::Failed(failure) = session.state() else {
        unreachable!();
    };
    assert_eq!(failure.kind, ErrorKind::NotFound);
}
