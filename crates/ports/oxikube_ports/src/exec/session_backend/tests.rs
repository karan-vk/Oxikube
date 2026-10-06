//! `SessionBackend` over hand-built `ExecSession`s.

use futures::channel::mpsc;
use futures::executor::block_on;
use futures::{SinkExt, StreamExt};
use oxikube_domain::{ErrorKind, OxiError};

use super::*;

struct Handles {
    stdin: mpsc::UnboundedReceiver<Vec<u8>>,
    resize: mpsc::UnboundedReceiver<TerminalSize>,
    stdout: mpsc::UnboundedSender<oxikube_domain::OxiResult<Vec<u8>>>,
    exit: oneshot::Sender<oxikube_domain::OxiResult<ExitStatus>>,
}

fn session(tty: bool) -> (SessionBackend, Handles) {
    let (stdin_tx, stdin) = mpsc::unbounded::<Vec<u8>>();
    let (resize_tx, resize) = mpsc::unbounded::<TerminalSize>();
    let (stdout_tx, stdout_rx) = mpsc::unbounded();
    let (exit, exit_rx) = oneshot::channel();
    let session = ExecSession {
        stdin: Some(Box::pin(
            stdin_tx.sink_map_err(|e| OxiError::network(e.to_string())),
        )),
        stdout: Some(Box::pin(stdout_rx)),
        stderr: None,
        resize: tty.then(|| {
            Box::pin(resize_tx.sink_map_err(|e| OxiError::network(e.to_string()))) as ResizeSink
        }),
        status: Box::pin(async move {
            exit_rx
                .await
                .unwrap_or_else(|_| Err(OxiError::network("gone")))
        }),
    };
    (
        SessionBackend::new(session),
        Handles {
            stdin,
            resize,
            stdout: stdout_tx,
            exit,
        },
    )
}

#[test]
fn writes_resizes_and_output_flow_through() {
    block_on(async {
        let (backend, mut h) = session(true);
        backend.write(b"ls\n").await.expect("write");
        backend
            .resize(TerminalSize::new(100, 30))
            .await
            .expect("resize");
        assert_eq!(h.stdin.next().await, Some(b"ls\n".to_vec()));
        assert_eq!(h.resize.next().await, Some(TerminalSize::new(100, 30)));

        let mut events = backend.output_stream();
        h.stdout.unbounded_send(Ok(b"hello".to_vec())).unwrap();
        match events.next().await {
            Some(BackendEvent::Output(bytes)) => assert_eq!(&bytes[..], b"hello"),
            other => panic!("unexpected {other:?}"),
        }
        h.stdout
            .unbounded_send(Err(OxiError::network("blip")))
            .unwrap();
        assert!(matches!(events.next().await, Some(BackendEvent::Error(_))));

        drop(h.stdout);
        h.exit.send(Ok(ExitStatus::with_code(3))).unwrap();
        match events.next().await {
            Some(BackendEvent::Exited(status)) => assert_eq!(status.code, Some(3)),
            other => panic!("unexpected {other:?}"),
        }
        assert!(events.next().await.is_none(), "Exited ends the stream");
    });
}

#[test]
fn output_stream_is_single_consumer() {
    block_on(async {
        let (backend, _h) = session(true);
        let _first = backend.output_stream();
        assert!(backend.output_stream().next().await.is_none());
    });
}

#[test]
fn kill_is_idempotent_and_ends_the_stream_with_a_signal() {
    block_on(async {
        let (backend, mut h) = session(true);
        let mut events = backend.output_stream();
        backend.kill().await.expect("kill");
        backend.kill().await.expect("kill again");
        match events.next().await {
            Some(BackendEvent::Exited(status)) => {
                assert_eq!(status.signal.as_deref(), Some("KILL"));
                assert!(!status.is_success());
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(events.next().await.is_none());
        // Stdin was closed.
        assert!(h.stdin.next().await.is_none());
        let err = backend.write(b"x").await.expect_err("closed");
        assert_eq!(err.kind(), ErrorKind::Conflict);
        let err = backend
            .resize(TerminalSize::new(1, 1))
            .await
            .expect_err("closed");
        assert_eq!(err.kind(), ErrorKind::Conflict);
    });
}

#[test]
fn kill_before_the_stream_is_taken_still_reports_the_end() {
    block_on(async {
        let (backend, _h) = session(false);
        backend.kill().await.expect("kill");
        // The stream was dropped with the connection: a late consumer sees it ended.
        assert!(backend.output_stream().next().await.is_none());
    });
}

#[test]
fn resize_without_a_tty_is_a_no_op() {
    block_on(async {
        let (backend, _h) = session(false);
        backend
            .resize(TerminalSize::new(80, 24))
            .await
            .expect("no-op");
    });
}

#[test]
fn a_session_without_stdin_refuses_writes() {
    block_on(async {
        let (_tx, rx) = mpsc::unbounded::<oxikube_domain::OxiResult<Vec<u8>>>();
        let backend = SessionBackend::new(ExecSession {
            stdin: None,
            stdout: Some(Box::pin(rx)),
            stderr: None,
            resize: None,
            status: Box::pin(async { Ok(ExitStatus::success()) }),
        });
        let err = backend.write(b"x").await.expect_err("no stdin");
        assert_eq!(err.kind(), ErrorKind::Unsupported);
    });
}

#[test]
fn dropping_the_backend_ends_the_stream() {
    block_on(async {
        let (backend, _h) = session(true);
        let mut events = backend.output_stream();
        drop(backend);
        assert!(matches!(
            events.next().await,
            Some(BackendEvent::Exited(status)) if status.signal.as_deref() == Some("KILL")
        ));
    });
}
