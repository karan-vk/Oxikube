//! `Parts` to `ExecSession`: streams, the exit status and ownership.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::channel::{mpsc, oneshot};
use futures::{SinkExt, StreamExt};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use kube::api::TerminalSize as KubeSize;
use oxikube_domain::ErrorKind;
use oxikube_ports::{ExitStatus, TerminalSize};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use crate::remote::exec::session::Parts;
use crate::remote::exec::status::exit_status;

/// Sets the flag when the future that holds it is dropped.
struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// The remote end of a [`Parts`]: what the websocket task would hold.
struct Remote {
    /// What the session wrote to stdin.
    stdin: DuplexStream,
    /// Where the process writes stdout.
    stdout: DuplexStream,
    resizes: mpsc::Receiver<KubeSize>,
    status: oneshot::Sender<Option<Status>>,
    finish: oneshot::Sender<Result<(), String>>,
    /// Set when the future that owns the "process" is dropped.
    process_dropped: Arc<AtomicBool>,
}

fn harness() -> (Parts, Remote) {
    let (stdin_w, stdin_r) = tokio::io::duplex(1024);
    let (stdout_w, stdout_r) = tokio::io::duplex(1024);
    let (resize_tx, resizes) = mpsc::channel(4);
    let (status_tx, status_rx) = oneshot::channel::<Option<Status>>();
    let (finish_tx, finish_rx) = oneshot::channel::<Result<(), String>>();
    let process_dropped = Arc::new(AtomicBool::new(false));
    let flag = DropFlag(process_dropped.clone());
    let parts = Parts {
        stdin: Some(Box::new(stdin_w)),
        stdout: Some(Box::new(stdout_r)),
        stderr: None,
        resize: Some(resize_tx),
        status: Box::pin(async move { status_rx.await.ok().flatten() }),
        finish: Box::pin(async move {
            let _process = flag;
            finish_rx.await.unwrap_or(Ok(()))
        }),
    };
    (
        parts,
        Remote {
            stdin: stdin_r,
            stdout: stdout_w,
            resizes,
            status: status_tx,
            finish: finish_tx,
            process_dropped,
        },
    )
}

fn failure(code: i32) -> Status {
    serde_json::from_value(json!({
        "status": "Failure",
        "reason": "NonZeroExitCode",
        "message": "command terminated with non-zero exit code: error executing command",
        "details": {"causes": [{"reason": "ExitCode", "message": code.to_string()}]},
    }))
    .expect("status")
}

#[tokio::test]
async fn a_session_exposes_exactly_the_streams_it_was_given() {
    let (mut parts, _remote) = harness();
    parts.stdin = None;
    parts.resize = None;
    let session = parts.into_session();
    assert!(session.stdin.is_none() && session.resize.is_none() && session.stderr.is_none());
    assert!(session.stdout.is_some());
}

#[tokio::test]
async fn bytes_flow_both_ways_and_resizes_arrive() {
    let (parts, mut remote) = harness();
    let mut session = parts.into_session();

    let stdin = session.stdin.as_mut().expect("stdin");
    stdin.send(b"ls\n".to_vec()).await.expect("send");
    let mut got = [0u8; 3];
    remote
        .stdin
        .read_exact(&mut got)
        .await
        .expect("remote read");
    assert_eq!(&got, b"ls\n");

    remote.stdout.write_all(b"file\n").await.expect("write");
    let chunk = session
        .stdout
        .as_mut()
        .expect("stdout")
        .next()
        .await
        .expect("chunk")
        .expect("ok");
    assert_eq!(chunk, b"file\n");

    let resize = session.resize.as_mut().expect("resize");
    resize
        .send(TerminalSize::new(100, 30))
        .await
        .expect("resize");
    let size = remote.resizes.next().await.expect("size");
    assert_eq!((size.width, size.height), (100, 30));

    // Closing the resize sink closes the channel; the session carries on.
    session
        .resize
        .as_mut()
        .expect("resize")
        .close()
        .await
        .expect("close");
    assert!(remote.resizes.next().await.is_none());
}

#[tokio::test]
async fn a_success_status_is_exit_code_zero() {
    let (parts, remote) = harness();
    let session = parts.into_session();
    let success: Status = serde_json::from_value(json!({"status": "Success"})).expect("status");
    remote.status.send(Some(success)).expect("send");
    remote.finish.send(Ok(())).expect("finish");
    assert_eq!(session.status.await.expect("status"), ExitStatus::success());
}

#[tokio::test]
async fn a_non_zero_exit_is_a_result_with_its_code() {
    let (parts, remote) = harness();
    let session = parts.into_session();
    remote.status.send(Some(failure(3))).expect("send");
    remote.finish.send(Ok(())).expect("finish");
    let status = session.status.await.expect("a result, not an error");
    assert_eq!(status.code, Some(3));
    assert!(!status.is_success());
    assert!(status.message.is_some());
}

#[test]
fn a_failure_without_an_exit_code_keeps_only_the_message() {
    let status: Status = serde_json::from_value(json!({
        "status": "Failure", "reason": "InternalError",
        "message": "error executing command: executable file not found in $PATH",
    }))
    .expect("status");
    let exit = exit_status(&status);
    assert_eq!(exit.code, None);
    assert!(
        exit.message
            .as_deref()
            .is_some_and(|m| m.contains("executable file not found"))
    );
}

#[tokio::test]
async fn a_connection_that_closes_without_a_status_has_no_exit_code() {
    let (parts, remote) = harness();
    let session = parts.into_session();
    drop(remote.status);
    remote.finish.send(Ok(())).expect("finish");
    let status = session.status.await.expect("a result");
    assert_eq!(status.code, None);
    assert!(status.message.is_some());
}

#[tokio::test]
async fn a_failed_connection_is_a_retryable_network_error() {
    let (parts, remote) = harness();
    let session = parts.into_session();
    drop(remote.status);
    remote
        .finish
        .send(Err("connection reset".into()))
        .expect("finish");
    let err = session.status.await.expect_err("an error");
    assert_eq!(err.kind(), ErrorKind::Network);
    assert!(err.is_retryable());
    assert!(err.message().contains("connection reset"));
}

#[tokio::test]
async fn dropping_the_session_drops_the_process() {
    let (parts, remote) = harness();
    let session = parts.into_session();
    assert!(!remote.process_dropped.load(Ordering::SeqCst));
    drop(session);
    assert!(
        remote.process_dropped.load(Ordering::SeqCst),
        "the websocket task is owned by the session's status future"
    );
}
