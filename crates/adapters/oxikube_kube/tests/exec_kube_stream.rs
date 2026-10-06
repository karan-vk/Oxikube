//! Kind integration for E09-S03: [`KubeStream`], the `TerminalBackend` over kube's
//! `AttachedProcess`. `echo hello` round-trips over `interactive_tty()`, `stty size` follows a
//! resize, a non-zero exit code propagates, a missing container and a missing `pods/exec`
//! permission are clear errors, `reconnect` opens a fresh shell on the same target, and opening
//! and dropping 50 sessions leaves no task behind. Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use k8s_openapi::api::rbac::v1::PolicyRule;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_kube::{KubeExec, KubeStream};
use oxikube_ports::{BackendEvent, ExecTarget, ExitStatus, TerminalBackend, TerminalSize};
use oxikube_testkit::integration::TestNamespace;

use common::exec::{OUTPUT_DEADLINE, create_sleeper};
use common::portforward::wait_ready;
use common::{DEADLINE, TestServiceAccount, wait_until};

fn shell(namespace: &str, pod: &str, command: &[&str]) -> ExecTarget {
    let pod = ResourceRef::new(
        ClusterId::new("kubeconfig", &ContextName::new("kind")),
        Gvk::new("", "v1", "Pod"),
        Some(namespace.into()),
        pod,
    );
    ExecTarget::interactive(pod, command.iter().map(|s| (*s).to_owned()).collect())
        .container("main")
}

/// Reads until the output contains `marker`; returns everything read so far.
async fn read_until(events: &mut BoxStream<'static, BackendEvent>, marker: &str) -> String {
    let mut seen = Vec::new();
    let found = tokio::time::timeout(OUTPUT_DEADLINE, async {
        while let Some(event) = events.next().await {
            match event {
                BackendEvent::Output(bytes) => seen.extend_from_slice(&bytes),
                other => panic!("ended early: {other:?}"),
            }
            if String::from_utf8_lossy(&seen).contains(marker) {
                return true;
            }
        }
        false
    })
    .await;
    let text = String::from_utf8_lossy(&seen).into_owned();
    assert_eq!(found, Ok(true), "never saw {marker:?}; got {text:?}");
    text
}

/// Drains the stream; returns the exit status it ended with.
async fn exit_of(events: &mut BoxStream<'static, BackendEvent>) -> ExitStatus {
    tokio::time::timeout(OUTPUT_DEADLINE, async {
        let mut exit = None;
        while let Some(event) = events.next().await {
            match event {
                BackendEvent::Exited(status) => exit = Some(status),
                BackendEvent::Error(err) => panic!("a drop instead of an exit: {err}"),
                BackendEvent::Output(_) => {}
            }
        }
        exit.expect("an exit event")
    })
    .await
    .expect("the stream ends")
}

/// A namespace with a running busybox pod `box` (container `main`) and an admin `KubeExec`.
async fn fixture() -> Option<(common::Kind, TestNamespace, KubeExec)> {
    let kind = common::kind().await?;
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_sleeper(&client, ns.name(), "box").await;
    wait_ready(&client, ns.name(), "box").await;
    let exec = KubeExec::new((*client).clone());
    Some((kind, ns, exec))
}

#[tokio::test]
async fn echo_round_trips_a_resize_reaches_stty_and_the_exit_code_propagates() {
    let Some((_kind, ns, exec)) = fixture().await else {
        return;
    };
    let stream: KubeStream = exec
        .exec_stream(&shell(ns.name(), "box", &["sh"]))
        .await
        .expect("exec");
    let mut events = stream.output_stream();

    // `hel""lo` prints `hello`; the TTY's echo of the line itself does not contain it.
    stream.write(b"echo hel\"\"lo\n").await.expect("write");
    read_until(&mut events, "hello\r\n").await;

    // The kubelet applies a resize asynchronously: ask until the shell sees it.
    for attempt in 1.. {
        stream
            .resize(TerminalSize::new(91, 27))
            .await
            .expect("resize");
        let line = format!("echo size=$(stty size)=en$((0))d{attempt}\n");
        stream.write(line.as_bytes()).await.expect("write");
        let text = read_until(&mut events, &format!("=en0d{attempt}")).await;
        if text.contains("size=27 91=en0d") {
            break;
        }
        assert!(attempt < 10, "the resize was never applied: {text:?}");
    }

    stream.write(b"exit 7\n").await.expect("write");
    let status = exit_of(&mut events).await;
    assert_eq!(status.code, Some(7), "{status:?}");
    assert!(!status.is_success());
}

#[tokio::test]
async fn a_missing_container_and_a_missing_permission_are_clear_errors() {
    let Some((kind, ns, exec)) = fixture().await else {
        return;
    };
    let ghost = shell(ns.name(), "box", &["sh"]).container("ghost");
    let err = exec
        .exec_stream(&ghost)
        .await
        .expect_err("no such container");
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");
    assert!(err.message().contains("ghost"), "{err}");

    // A reader may read pods but not open exec streams.
    let rules = vec![PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec!["pods".into()]),
        verbs: vec!["get".into(), "list".into()],
        ..PolicyRule::default()
    }];
    let admin = kind.admin_client().await;
    let reader = TestServiceAccount::create(&admin, ns.name(), "reader", rules).await;
    let name = "kube-stream-reader";
    let pool = kind.pool(kind.with_token_context(name, &reader.token));
    let client = pool.get(&name.into()).await.expect("reader client");
    let denied = KubeExec::new((*client).clone());
    let target = shell(ns.name(), "box", &["sh"]);
    let err = wait_until("the role is effective", DEADLINE, || async {
        denied
            .exec_stream(&target)
            .await
            .err()
            .filter(|e| e.kind() == ErrorKind::Forbidden)
    })
    .await;
    assert!(err.message().contains("pods/exec"), "{err}");
}

#[tokio::test]
async fn reconnect_opens_a_fresh_shell_on_the_same_target() {
    let Some((_kind, ns, exec)) = fixture().await else {
        return;
    };
    let first = exec
        .exec_stream(&shell(ns.name(), "box", &["sh"]))
        .await
        .expect("exec");
    let mut events = first.output_stream();
    first.write(b"export MARK=old\n").await.expect("write");
    first.write(b"echo mark=$MARK.\n").await.expect("write");
    read_until(&mut events, "mark=old.").await;
    first.kill().await.expect("kill");

    let second = first.reconnect().await.expect("reconnect");
    drop(first);
    let mut events = second.output_stream();
    second
        .write(b"echo mark=${MARK:-fresh}.\n")
        .await
        .expect("write");
    read_until(&mut events, "mark=fresh.").await;
    second.write(b"exit\n").await.expect("write");
    assert!(exit_of(&mut events).await.is_success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_and_dropping_fifty_sessions_leaves_no_tasks() {
    let Some((_kind, ns, exec)) = fixture().await else {
        return;
    };
    let target = shell(ns.name(), "box", &["cat"]);
    let metrics = tokio::runtime::Handle::current().metrics();
    let open_one = |round: usize| {
        let exec = exec.clone();
        let target = target.clone();
        async move {
            let stream = exec.exec_stream(&target).await.expect("exec");
            let mut events = stream.output_stream();
            stream
                .write(format!("ping-{round}\n").as_bytes())
                .await
                .expect("write");
            read_until(&mut events, &format!("ping-{round}")).await;
            // The terminal's pump runs on the runtime, like the view's would.
            tokio::spawn(async move { while events.next().await.is_some() {} });
            drop(stream);
        }
    };
    // One warm-up so the client's own connection tasks are in the baseline.
    open_one(0).await;
    let settle = || async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        metrics.num_alive_tasks()
    };
    let baseline = settle().await;
    for round in 1..=50 {
        open_one(round).await;
    }
    let alive = tokio::time::timeout(DEADLINE, async {
        loop {
            let alive = settle().await;
            if alive <= baseline {
                return alive;
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "tasks grew: {} alive after 50 sessions, {baseline} before",
            metrics.num_alive_tasks()
        )
    });
    assert!(alive <= baseline);
}

#[tokio::test]
async fn a_flood_of_output_arrives_whole_and_reports_its_throughput() {
    let Some((_kind, ns, exec)) = fixture().await else {
        return;
    };
    const SIZE: usize = 16 * 1024 * 1024;
    let flood = ExecTarget {
        tty: false,
        stdin: false,
        ..shell(ns.name(), "box", &["sh", "-c", "yes | head -c 16777216"])
    };
    let started = std::time::Instant::now();
    let stream = exec.exec_stream(&flood).await.expect("exec");
    let mut events = stream.output_stream();
    let (mut received, mut chunks) = (0usize, 0usize);
    let exit = tokio::time::timeout(OUTPUT_DEADLINE, async {
        while let Some(event) = events.next().await {
            match event {
                BackendEvent::Output(bytes) => {
                    received += bytes.len();
                    chunks += 1;
                }
                BackendEvent::Exited(exit) => return exit,
                BackendEvent::Error(err) => panic!("{err}"),
            }
        }
        panic!("ended without an exit");
    })
    .await
    .expect("the flood ends");
    let elapsed = started.elapsed();
    assert!(exit.is_success(), "{exit:?}");
    assert_eq!(received, SIZE);
    eprintln!(
        "perf: {} MiB in {elapsed:?} ({:.1} MiB/s), {chunks} chunks, {} B mean chunk",
        SIZE >> 20,
        (SIZE as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64(),
        received / chunks.max(1)
    );
}
