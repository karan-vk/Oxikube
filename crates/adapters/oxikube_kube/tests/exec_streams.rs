//! Kind integration for E04-S09 exec and attach: echo round trip over a TTY (with latency),
//! a large payload, resize observed with `stty size`, exit status, stderr, attach, and the
//! error kinds. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly
//! otherwise.
#![cfg(feature = "integration")]

mod common;

use std::time::{Duration, Instant};

use futures::SinkExt;
use k8s_openapi::api::rbac::v1::PolicyRule;
use oxikube_domain::ErrorKind;
use oxikube_kube::KubeExec;
use oxikube_ports::{ExecOptions, ExecPort, TerminalSize};
use oxikube_testkit::integration::TestNamespace;

use common::exec::{create_cat, create_sleeper, read_all, read_until};
use common::portforward::wait_ready;
use common::{DEADLINE, TestServiceAccount, wait_until};

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

#[tokio::test]
async fn a_tty_shell_echoes_and_reports_the_round_trip_latency() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_sleeper(&client, ns.name(), "box").await;
    wait_ready(&client, ns.name(), "box").await;
    let exec = KubeExec::new((*client).clone());

    let mut session = exec
        .exec(
            ns.name(),
            "box",
            &argv(&["sh"]),
            &ExecOptions::interactive().container("main"),
        )
        .await
        .expect("exec");
    let mut stdin = session.stdin.take().expect("stdin");
    let mut stdout = session.stdout.take().expect("stdout");
    assert!(session.stderr.is_none(), "a TTY merges stderr into stdout");

    // The tty echoes the typed line, so the marker is built by the shell: only its output
    // contains the computed value.
    stdin
        .send(b"echo oxi-$((40+2))\n".to_vec())
        .await
        .expect("send");
    let first = read_until(&mut stdout, "oxi-42").await;
    assert!(
        first.contains("echo oxi-$((40+2))"),
        "the tty echoes the input: {first:?}"
    );

    let mut samples = Vec::new();
    for round in 0..20 {
        let marker = format!("rt-{}", 1000 + round);
        let started = Instant::now();
        stdin
            .send(format!("echo rt-$((1000+{round}))\n").into_bytes())
            .await
            .expect("send");
        read_until(&mut stdout, &marker).await;
        samples.push(started.elapsed());
    }
    samples.sort();
    eprintln!(
        "exec echo round trip over a TTY: median {:?}, p95 {:?}, max {:?}",
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100],
        samples[samples.len() - 1]
    );
    assert!(samples[samples.len() / 2] < Duration::from_millis(500));

    stdin.send(b"exit\n".to_vec()).await.expect("send");
    let status = session.status.await.expect("status");
    assert!(status.is_success(), "{status:?}");
}

#[tokio::test]
async fn a_large_payload_round_trips_through_cat() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_sleeper(&client, ns.name(), "box").await;
    wait_ready(&client, ns.name(), "box").await;
    let exec = KubeExec::new((*client).clone());

    let options = ExecOptions {
        container: Some("main".into()),
        stdin: true,
        stdout: true,
        stderr: false,
        tty: false,
    };
    let mut session = exec
        .exec(ns.name(), "box", &argv(&["cat"]), &options)
        .await
        .expect("exec");
    let mut stdin = session.stdin.take().expect("stdin");
    let mut stdout = session.stdout.take().expect("stdout");

    let payload: Vec<u8> = (0..1_048_576u32)
        .map(|i| (i.wrapping_mul(31) % 251) as u8)
        .collect();
    let sent = payload.clone();
    let writer = tokio::spawn(async move {
        for chunk in sent.chunks(8192) {
            stdin.send(chunk.to_vec()).await.expect("send");
        }
        // Closing the sink is the end of the input: cat then exits.
        stdin.close().await.expect("close");
    });
    let started = Instant::now();
    let echoed = read_all(&mut stdout).await;
    eprintln!(
        "exec 1 MiB through cat: {:?} ({:.1} MiB/s each way)",
        started.elapsed(),
        1.0 / started.elapsed().as_secs_f64()
    );
    writer.await.expect("writer");
    assert_eq!(echoed.len(), payload.len());
    assert!(
        echoed == payload,
        "the echoed bytes differ from the payload"
    );
    assert!(session.status.await.expect("status").is_success());
}

#[tokio::test]
async fn a_resize_is_observed_by_stty() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_sleeper(&client, ns.name(), "box").await;
    wait_ready(&client, ns.name(), "box").await;
    let exec = KubeExec::new((*client).clone());

    let mut session = exec
        .exec(
            ns.name(),
            "box",
            &argv(&["sh"]),
            &ExecOptions::interactive().container("main"),
        )
        .await
        .expect("exec");
    let mut stdin = session.stdin.take().expect("stdin");
    let mut stdout = session.stdout.take().expect("stdout");
    let mut resize = session
        .resize
        .take()
        .expect("a TTY session has a resize sink");

    // The kubelet applies a resize asynchronously to the shell's input, so ask until it is
    // visible instead of assuming an order.
    for (width, height) in [(100u16, 30u16), (61, 17)] {
        let expected = format!("{height} {width}");
        let mut attempts = 0;
        loop {
            attempts += 1;
            resize
                .send(TerminalSize::new(width, height))
                .await
                .expect("resize");
            stdin
                .send(format!("echo size=$(stty size)=end{attempts}\n").into_bytes())
                .await
                .expect("send");
            let seen = read_until(&mut stdout, &format!("=end{attempts}")).await;
            if seen.contains(&format!("size={expected}=end")) {
                break;
            }
            assert!(attempts < 10, "stty never reported {expected}: {seen:?}");
        }
    }
    resize
        .close()
        .await
        .expect("closing the resize sink is clean");
    // The session still works after the resize channel closed.
    stdin
        .send(b"echo still-$((1+1))\n".to_vec())
        .await
        .expect("send");
    read_until(&mut stdout, "still-2").await;
    stdin.send(b"exit\n".to_vec()).await.expect("send");
    assert!(session.status.await.expect("status").is_success());
}

#[tokio::test]
async fn the_exit_status_and_separate_stderr_are_propagated() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_sleeper(&client, ns.name(), "box").await;
    wait_ready(&client, ns.name(), "box").await;
    let exec = KubeExec::new((*client).clone());
    let options = ExecOptions::default().container("main");

    let mut session = exec
        .exec(
            ns.name(),
            "box",
            &argv(&["sh", "-c", "echo to-out; echo to-err >&2; exit 3"]),
            &options,
        )
        .await
        .expect("exec");
    let out = read_all(&mut session.stdout.take().expect("stdout")).await;
    let err = read_all(&mut session.stderr.take().expect("stderr")).await;
    assert_eq!(String::from_utf8_lossy(&out).trim(), "to-out");
    assert_eq!(String::from_utf8_lossy(&err).trim(), "to-err");
    let status = session.status.await.expect("a non-zero exit is a result");
    assert_eq!(status.code, Some(3), "{status:?}");
    assert!(!status.is_success());

    let ok = exec
        .exec(ns.name(), "box", &argv(&["true"]), &options)
        .await
        .expect("exec");
    assert!(ok.status.await.expect("status").is_success());

    // The command is not run through a shell: a missing program is a failure result.
    let missing = exec
        .exec(ns.name(), "box", &argv(&["no-such-program"]), &options)
        .await
        .expect("the stream opens");
    let status = missing.status.await.expect("a result");
    assert!(!status.is_success(), "{status:?}");
}

#[tokio::test]
async fn attach_talks_to_the_main_process() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_cat(&client, ns.name(), "cat").await;
    wait_ready(&client, ns.name(), "cat").await;
    let exec = KubeExec::new((*client).clone());

    let options = ExecOptions {
        container: Some("main".into()),
        stdin: true,
        stdout: true,
        stderr: false,
        tty: false,
    };
    let mut session = exec
        .attach(ns.name(), "cat", &options)
        .await
        .expect("attach");
    let mut stdin = session.stdin.take().expect("stdin");
    let mut stdout = session.stdout.take().expect("stdout");
    stdin.send(b"attached-ping\n".to_vec()).await.expect("send");
    read_until(&mut stdout, "attached-ping").await;
    // Dropping the session closes the connection without ending the pod's process.
    drop(stdin);
    drop(stdout);
    drop(session);
    let again = exec
        .attach(ns.name(), "cat", &options)
        .await
        .expect("attach again");
    drop(again);
}

#[tokio::test]
async fn failures_map_to_the_error_kinds() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    create_sleeper(&admin, ns.name(), "box").await;
    wait_ready(&admin, ns.name(), "box").await;
    let exec = KubeExec::new((*admin).clone());
    let options = ExecOptions::default();

    let err = exec
        .exec(ns.name(), "ghost", &argv(&["true"]), &options)
        .await
        .expect_err("no such pod");
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");

    let err = exec
        .exec(
            ns.name(),
            "box",
            &argv(&["true"]),
            &options.clone().container("nope"),
        )
        .await
        .expect_err("no such container");
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");
    assert!(err.message().contains("nope"), "{err}");

    // A reader may read pods but not open streams.
    let rules = vec![PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec!["pods".into()]),
        verbs: vec!["get".into(), "list".into()],
        ..PolicyRule::default()
    }];
    let reader = TestServiceAccount::create(&admin, ns.name(), "reader", rules).await;
    let name = "exec-reader";
    let pool = kind.pool(kind.with_token_context(name, &reader.token));
    let client = pool.get(&name.into()).await.expect("reader client");
    let denied = KubeExec::new((*client).clone());
    let err = wait_until("the role is effective", DEADLINE, || async {
        denied
            .exec(ns.name(), "box", &argv(&["true"]), &options)
            .await
            .err()
            .filter(|e| e.kind() == ErrorKind::Forbidden)
    })
    .await;
    assert!(err.message().contains("pods/exec"), "{err}");
}
