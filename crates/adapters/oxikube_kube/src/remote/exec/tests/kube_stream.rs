//! `KubeStream`: the session driven as a `TerminalBackend` over the same `Parts` the websocket
//! fills (in-memory pipes stand in for it), the end of a session (exit or drop), ownership, and
//! `reconnect` against the testkit's `FakeExecStreamPort` and the fake API. The live websocket
//! is `tests/exec_kube_stream.rs` on kind.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{
    BackendEvent, ExecOptions, ExecStreamPort, ExecTarget, ExitStatus, TerminalBackend,
    TerminalSize,
};
use oxikube_testkit::fakes::{ExecScript, ExecStreamCall, FakeExecStreamPort};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::session::{Remote, failure, harness};
use crate::fake_api::{FakeApi, status_body};
use crate::remote::exec::KubeExec;
use crate::remote::exec::kube_stream::{KubeStream, Reopen};
use crate::remote::exec::params::attach_params;

const DEADLINE: Duration = Duration::from_secs(5);

fn sh() -> Vec<String> {
    vec!["sh".into()]
}

fn tty() -> ExecOptions {
    ExecOptions::interactive().container("app")
}

/// A backend over in-memory pipes, reopened through `port` as an exec of `sh`.
fn stream(port: Arc<dyn ExecStreamPort>) -> (KubeStream, Remote) {
    let (parts, remote) = harness();
    let reopen = Reopen::exec(port, "default", "p", &sh(), &tty());
    (KubeStream::new(parts.into_session(), Some(reopen)), remote)
}

fn unscripted() -> Arc<dyn ExecStreamPort> {
    Arc::new(FakeExecStreamPort::new())
}

/// Everything until the stream ends: the output, then the last event (`Exited` or `Error`).
async fn drain(events: &mut BoxStream<'static, BackendEvent>) -> (Vec<u8>, BackendEvent) {
    tokio::time::timeout(DEADLINE, async {
        let mut output = Vec::new();
        let mut last = None;
        while let Some(event) = events.next().await {
            match event {
                BackendEvent::Output(bytes) => output.extend_from_slice(&bytes),
                other => last = Some(other),
            }
        }
        (
            output,
            last.expect("the stream ends with an exit or an error"),
        )
    })
    .await
    .expect("the stream ends")
}

fn success() -> Status {
    serde_json::from_value(json!({"status": "Success"})).expect("status")
}

#[tokio::test]
async fn stdin_stdout_and_resize_flow_through_the_backend() {
    let (backend, mut remote) = stream(unscripted());
    let mut events = backend.output_stream();

    backend.write(b"echo hi\n").await.expect("write");
    let mut typed = [0u8; 8];
    remote.stdin.read_exact(&mut typed).await.expect("stdin");
    assert_eq!(&typed, b"echo hi\n");

    remote.stdout.write_all(b"first ").await.expect("stdout");
    let Some(BackendEvent::Output(first)) = events.next().await else {
        panic!("output first");
    };
    remote.stdout.write_all(b"second").await.expect("stdout");
    let Some(BackendEvent::Output(second)) = events.next().await else {
        panic!("output in order");
    };
    assert_eq!((&first[..], &second[..]), (&b"first "[..], &b"second"[..]));

    backend
        .resize(TerminalSize::new(132, 43).with_pixels(1000, 800))
        .await
        .expect("resize");
    let size = remote.resizes.next().await.expect("a resize");
    assert_eq!((size.width, size.height), (132, 43), "pixels are not sent");
}

#[tokio::test]
async fn the_status_channel_becomes_the_exit_event() {
    for (status, code) in [(success(), 0), (failure(3), 3), (failure(137), 137)] {
        let (backend, mut remote) = stream(unscripted());
        let mut events = backend.output_stream();
        remote.stdout.write_all(b"bye\n").await.expect("stdout");
        drop(remote.stdout);
        remote.status.send(Some(status)).expect("status");
        remote.finish.send(Ok(())).expect("finish");
        let (output, last) = drain(&mut events).await;
        assert_eq!(output, b"bye\n");
        let BackendEvent::Exited(exit) = last else {
            panic!("an exit, got {last:?}");
        };
        assert_eq!(exit.code, Some(code));
        assert_eq!(exit.is_success(), code == 0);
    }
}

#[tokio::test]
async fn a_close_without_a_status_is_a_disconnect_not_an_exit() {
    let (backend, remote) = stream(unscripted());
    let mut events = backend.output_stream();
    drop(remote.stdout);
    drop(remote.status);
    remote.finish.send(Ok(())).expect("finish");
    let (_, last) = drain(&mut events).await;
    let BackendEvent::Error(err) = last else {
        panic!("a disconnect, got {last:?}");
    };
    assert_eq!(err.kind(), ErrorKind::Network, "{err}");
    assert!(err.is_retryable(), "the cue for Reconnect");
    assert!(backend.can_reconnect());
}

#[tokio::test]
async fn a_websocket_failure_is_a_disconnect() {
    let (backend, remote) = stream(unscripted());
    let mut events = backend.output_stream();
    drop(remote.stdout);
    drop(remote.status);
    remote
        .finish
        .send(Err("connection reset by peer".into()))
        .expect("finish");
    let (_, last) = drain(&mut events).await;
    assert!(
        matches!(&last, BackendEvent::Error(err) if err.kind() == ErrorKind::Network && err.is_retryable()),
        "{last:?}"
    );
}

#[tokio::test]
async fn kill_ends_the_stream_with_kill_and_closes_the_connection() {
    let (backend, remote) = stream(unscripted());
    let mut events = backend.output_stream();
    backend.kill().await.expect("kill");
    backend.kill().await.expect("idempotent");
    let (_, last) = drain(&mut events).await;
    assert!(
        matches!(&last, BackendEvent::Exited(exit) if exit.signal.as_deref() == Some("KILL")),
        "{last:?}"
    );
    assert!(remote.process_dropped.load(Ordering::SeqCst), "aborted");
    let err = backend.write(b"ls\n").await.expect_err("closed");
    assert_eq!(err.kind(), ErrorKind::Conflict);
}

#[tokio::test]
async fn dropping_the_backend_stops_a_running_consumer_and_the_process() {
    let (backend, remote) = stream(unscripted());
    let mut events = backend.output_stream();
    // The terminal's pump: reads until the stream ends.
    let pump = tokio::spawn(async move { drain(&mut events).await.1 });
    tokio::task::yield_now().await;
    drop(backend);
    let last = tokio::time::timeout(DEADLINE, pump)
        .await
        .expect("the pump ends")
        .expect("no panic");
    assert!(matches!(last, BackendEvent::Exited(_)), "{last:?}");
    assert!(remote.process_dropped.load(Ordering::SeqCst), "aborted");
}

#[tokio::test]
async fn opening_and_dropping_fifty_sessions_leaves_no_tasks() {
    let metrics = tokio::runtime::Handle::current().metrics();
    let baseline = metrics.num_alive_tasks();
    let mut remotes = Vec::new();
    for _ in 0..50 {
        let (backend, remote) = stream(unscripted());
        let mut events = backend.output_stream();
        tokio::spawn(async move { while events.next().await.is_some() {} });
        backend.write(b"x").await.expect("write");
        drop(backend);
        remotes.push(remote);
    }
    tokio::time::timeout(DEADLINE, async {
        while metrics.num_alive_tasks() > baseline {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("every pump ended");
    assert!(
        remotes
            .iter()
            .all(|r| r.process_dropped.load(Ordering::SeqCst))
    );
}

#[tokio::test]
async fn reconnect_opens_the_same_exec_again() {
    let port = Arc::new(FakeExecStreamPort::new());
    port.script().exec.push(Ok(ExecScript::new()
        .stdout("again\n")
        .exit(Ok(ExitStatus::success()))));
    let (backend, _remote) = stream(port.clone());

    let fresh = backend.reconnect().await.expect("reconnect");
    assert!(
        fresh.can_reconnect(),
        "a reconnected session can reconnect again"
    );
    let (output, last) = drain(&mut fresh.output_stream()).await;
    assert_eq!(output, b"again\n");
    assert!(matches!(last, BackendEvent::Exited(exit) if exit.is_success()));
    assert_eq!(
        port.recorded_calls(),
        vec![ExecStreamCall::Exec {
            namespace: "default".into(),
            pod: "p".into(),
            command: sh(),
            options: tty(),
        }]
    );
}

#[tokio::test]
async fn reconnect_attaches_again_and_reports_a_failure() {
    let port = Arc::new(FakeExecStreamPort::new());
    port.script()
        .attach
        .push(Err(OxiError::not_found("pod default/p not found")));
    let (parts, _remote) = harness();
    let reopen = Reopen::attach(port.clone(), "default", "p", &tty());
    let backend = KubeStream::new(parts.into_session(), Some(reopen));
    let err = backend.reconnect().await.expect_err("the pod is gone");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(matches!(
        &port.recorded_calls()[..],
        [ExecStreamCall::Attach { pod, options, .. }] if pod == "p" && *options == tty()
    ));
}

#[tokio::test]
async fn a_session_without_a_target_cannot_reconnect() {
    let (parts, _remote) = harness();
    let backend = KubeStream::new(parts.into_session(), None);
    assert!(!backend.can_reconnect());
    let err = backend.reconnect().await.expect_err("node shell");
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

#[tokio::test]
async fn reconnect_sends_the_same_request_to_the_cluster() {
    const EXEC: &str = "/api/v1/namespaces/default/pods/p/exec";
    let api = FakeApi::new();
    api.reply(EXEC, 403, status_body(403, "Forbidden", "no"));
    api.reply(EXEC, 403, status_body(403, "Forbidden", "no"));
    let exec = KubeExec::new(api.client());
    let target = ExecTarget::interactive(
        oxikube_domain::ids::ResourceRef::new(
            oxikube_domain::ids::ClusterId::new(
                "kubeconfig",
                &oxikube_domain::ids::ContextName::new("kind"),
            ),
            oxikube_domain::ids::Gvk::new("", "v1", "Pod"),
            Some("default".into()),
            "p",
        ),
        sh(),
    )
    .container("app");
    let err = exec.exec_stream(&target).await.expect_err("refused");
    assert_eq!(err.kind(), ErrorKind::Forbidden);

    let (parts, _remote) = harness();
    let reopen = Reopen::exec(Arc::new(exec), "default", "p", &sh(), &tty());
    let backend = KubeStream::new(parts.into_session(), Some(reopen));
    let err = backend.reconnect().await.expect_err("still refused");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    let requests = api.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, requests[1].path);
    assert_eq!(requests[0].query, requests[1].query, "the same target");
}

#[test]
fn a_terminal_uses_kubes_interactive_tty_params() {
    let params = attach_params(&tty());
    let expected = kube::api::AttachParams::interactive_tty().container("app");
    assert_eq!(
        (params.stdin, params.stdout, params.stderr, params.tty),
        (
            expected.stdin,
            expected.stdout,
            expected.stderr,
            expected.tty
        )
    );
    assert_eq!(params.container, expected.container);
    assert_eq!(
        params.max_stdout_buf_size,
        Some(crate::remote::exec::params::STREAM_BUFFER)
    );
    let plain = attach_params(&ExecOptions::default());
    assert!(!plain.tty && !plain.stdin && plain.stdout && plain.stderr);
}

#[test]
fn debug_output_names_the_target_but_never_the_command() {
    let (parts, _remote) = harness();
    let secret = vec!["sh".into(), "-c".into(), "TOKEN=hunter2 run".into()];
    let reopen = Reopen::exec(unscripted(), "default", "p", &secret, &tty());
    let backend = KubeStream::new(parts.into_session(), Some(reopen));
    let debug = format!("{backend:?}");
    assert!(
        debug.contains("default") && debug.contains("\"p\""),
        "{debug}"
    );
    assert!(!debug.contains("hunter2"), "{debug}");
}
