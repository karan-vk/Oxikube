//! Uploads: stdin fed and closed, exits mapped, cancel, source failures, binary safety.

use std::sync::Arc;

use bytes::Bytes;
use futures::executor::block_on;
use futures::stream;
use futures::{FutureExt as _, StreamExt as _};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use parking_lot::Mutex;

use super::super::{
    ByteStream, CancellationToken, ExecTransferExt, RemotePath, TransferOptions, TransferProgress,
};
use super::{Plan, Scripted, Stdin};

fn dest() -> RemotePath {
    RemotePath::new("demo", "web", "/srv/in")
}

fn archive(chunks: Vec<Vec<u8>>) -> ByteStream {
    stream::iter(chunks.into_iter().map(|chunk| Ok(Bytes::from(chunk)))).boxed()
}

fn with_stdin(plan: Plan, stdin: &Stdin) -> Scripted {
    Scripted::with(Plan {
        stdin: Some(stdin.sink()),
        ..plan
    })
}

#[test]
fn the_archive_reaches_stdin_which_is_then_closed() {
    let all: Vec<u8> = (0..=255u8).cycle().take(100_000).collect();
    let stdin = Stdin::new();
    let port = with_stdin(Plan::exits(0), &stdin);
    let reports = Arc::new(Mutex::new(Vec::<TransferProgress>::new()));
    let sink = reports.clone();
    let options = TransferOptions {
        progress: Some(Arc::new(move |progress| sink.lock().push(progress))),
        ..TransferOptions::default()
    };
    let chunks = all.chunks(32 * 1024).map(<[u8]>::to_vec).collect();
    let sent = block_on(port.tar_upload(&dest(), archive(chunks), options)).expect("uploaded");
    assert_eq!(sent, 100_000);
    assert_eq!(
        *stdin.bytes.lock(),
        all,
        "every byte value arrives unchanged"
    );
    assert!(
        stdin.closed(),
        "stdin is closed so tar sees the end of input"
    );
    assert_eq!(reports.lock().last().map(|p| p.bytes), Some(100_000));
    let options = port.seen.options.lock().clone().expect("opened");
    assert!(options.stdin && !options.tty && options.stderr);
    assert_eq!(
        *port.seen.command.lock(),
        ["tar", "-xmf", "-", "-C", "/srv/in"]
            .map(str::to_owned)
            .to_vec()
    );
}

#[test]
fn a_failed_extract_is_typed_from_stderr() {
    let stdin = Stdin::new();
    let port = with_stdin(
        Plan::exits(2).stderr("tar: can't create directory 'x': Read-only file system\n"),
        &stdin,
    );
    let error = block_on(port.tar_upload(
        &dest(),
        archive(vec![vec![0; 10]]),
        TransferOptions::default(),
    ))
    .expect_err("refused");
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert!(error.message().contains("Read-only file system"), "{error}");

    let stdin = Stdin::new();
    let port = with_stdin(Plan::exits(126), &stdin);
    let error = block_on(port.tar_upload(
        &dest(),
        archive(vec![vec![0; 10]]),
        TransferOptions::default(),
    ))
    .expect_err("no tar");
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message().contains("pod demo/web"), "{error}");
}

#[test]
fn tar_that_stops_reading_after_the_end_marker_still_succeeds() {
    let stdin = Stdin::new().accepting(1024);
    let port = with_stdin(Plan::exits(0), &stdin);
    let sent = block_on(port.tar_upload(
        &dest(),
        archive(vec![vec![1; 1024], vec![0; 9216]]),
        TransferOptions::default(),
    ))
    .expect("tar exited 0");
    assert_eq!(sent, 1024);
}

#[test]
fn a_remote_that_stops_reading_and_fails_reports_its_error() {
    let stdin = Stdin::new().accepting(0);
    let port = with_stdin(Plan::exits(1).stderr("tar: invalid tar magic\n"), &stdin);
    let error = block_on(port.tar_upload(
        &dest(),
        archive(vec![vec![1; 10]]),
        TransferOptions::default(),
    ))
    .expect_err("bad archive");
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert!(error.message().contains("invalid tar magic"), "{error}");
}

#[test]
fn a_failing_source_abandons_the_upload_and_aborts_the_session() {
    let stdin = Stdin::new();
    let port = with_stdin(Plan::exits(0).running(), &stdin);
    let source: Vec<OxiResult<Bytes>> = vec![
        Ok(Bytes::from_static(b"head")),
        Err(OxiError::internal("disk read failed")),
    ];
    let error = block_on(port.tar_upload(
        &dest(),
        stream::iter(source).boxed(),
        TransferOptions::default(),
    ))
    .expect_err("source failed");
    assert_eq!(error.message(), "disk read failed");
    assert!(
        !stdin.closed(),
        "no end of input is sent for a broken archive"
    );
    assert!(port.seen.aborted());
}

#[test]
fn cancel_stops_a_blocked_upload_and_aborts_the_session() {
    let stdin = Stdin::new().blocked();
    let port = with_stdin(Plan::exits(0).running(), &stdin);
    let cancel = CancellationToken::new();
    let options = TransferOptions {
        cancel: cancel.clone(),
        ..TransferOptions::default()
    };
    let target = dest();
    let mut upload = port.tar_upload(&target, archive(vec![vec![1; 10]]), options);
    assert!(
        (&mut upload).now_or_never().is_none(),
        "waiting for the remote to read"
    );
    assert!(!port.seen.aborted());
    cancel.cancel();
    let error = block_on(upload).expect_err("cancelled");
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert!(cancel.is_cancelled());
    assert!(port.seen.aborted());
}
