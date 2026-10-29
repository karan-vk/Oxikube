//! Downloads: chunks, stderr, exits, cancel, memory, binary safety.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::executor::block_on;
use futures::stream;
use futures::{FutureExt as _, StreamExt as _};
use oxikube_domain::{ErrorKind, OxiError};
use parking_lot::Mutex;

use super::super::super::ExitStatus;
use super::super::{
    CancellationToken, ExecTransferExt, RemotePath, TransferOptions, TransferProgress,
};
use super::{Plan, Scripted, Stdin};

fn source() -> RemotePath {
    RemotePath::new("demo", "web", "/var/data").in_container("app")
}

#[test]
fn chunks_pass_through_unchanged_and_the_session_has_no_tty() {
    let port = Scripted::with(Plan::exits(0).stdout_chunks(vec![
        b"abc".to_vec(),
        vec![0u8; 70_000],
        b"z".to_vec(),
    ]));
    let chunks: Vec<_> = block_on(async {
        let stream = port
            .tar_download(&source(), TransferOptions::default())
            .await
            .expect("open");
        stream
            .map(|chunk| chunk.expect("chunk").len())
            .collect()
            .await
    });
    assert_eq!(chunks, vec![3, 70_000, 1]);
    let options = port.seen.options.lock().clone().expect("opened");
    assert!(!options.tty && options.stdout && options.stderr);
    assert!(
        options.stdin,
        "stdin stays open: its end stops the remote tar"
    );
    assert_eq!(options.container.as_deref(), Some("app"));
    assert_eq!(port.seen.command.lock()[4..7], ["tar", "-cf", "-"]);
}

#[test]
fn every_byte_value_survives() {
    let all: Vec<u8> = (0..=255u8).cycle().take(256 * 40).collect();
    let chunks: Vec<Vec<u8>> = all.chunks(333).map(<[u8]>::to_vec).collect();
    let port = Scripted::with(Plan::exits(0).stdout_chunks(chunks));
    let got: Vec<u8> = block_on(async {
        let stream = port
            .cat_download(&source(), TransferOptions::default())
            .await
            .expect("open");
        stream
            .map(|chunk| chunk.expect("chunk").to_vec())
            .concat()
            .await
    });
    assert_eq!(got, all);
    assert_eq!(
        *port.seen.command.lock(),
        vec!["cat".to_owned(), "/var/data".to_owned()]
    );
}

#[test]
fn a_warning_on_stderr_with_exit_zero_is_a_success() {
    let port = Scripted::with(
        Plan::exits(0)
            .stdout_chunks(vec![b"archive".to_vec()])
            .stderr("tar: Removing leading `/' from member names\n"),
    );
    let items: Vec<_> = block_on(async {
        port.tar_download(&source(), TransferOptions::default())
            .await
            .expect("open")
            .collect()
            .await
    });
    assert_eq!(items.len(), 1);
    assert!(items[0].is_ok());
}

fn failure_of(plan: Plan) -> OxiError {
    let port = Scripted::with(plan);
    let items: Vec<_> = block_on(async {
        port.tar_download(&source(), TransferOptions::default())
            .await
            .expect("open")
            .collect()
            .await
    });
    let last = items.into_iter().last().expect("an item");
    last.expect_err("the download fails")
}

#[test]
fn non_zero_exits_are_typed_by_what_tar_said() {
    let missing =
        failure_of(Plan::exits(1).stderr("tar: can't open 'nope': No such file or directory\n"));
    assert_eq!(missing.kind(), ErrorKind::NotFound);
    assert!(missing.message().contains("/var/data"), "{missing}");
    assert!(
        missing.message().contains("container app of pod demo/web"),
        "{missing}"
    );

    let denied = failure_of(Plan::exits(2).stderr("tar: secret: Cannot open: Permission denied\n"));
    assert_eq!(denied.kind(), ErrorKind::Forbidden);

    let other = failure_of(Plan::exits(2).stderr("tar: something odd\n"));
    assert_eq!(other.kind(), ErrorKind::Conflict);
    assert!(other.message().contains("exit code 2"), "{other}");
    assert!(other.message().contains("something odd"), "{other}");
}

#[test]
fn a_missing_tar_is_unsupported_and_names_the_container() {
    let by_code = failure_of(Plan::exits(127));
    assert_eq!(by_code.kind(), ErrorKind::Unsupported);
    assert!(
        by_code
            .message()
            .contains("tar is not available in container app"),
        "{by_code}"
    );

    let by_message = failure_of(Plan {
        status: Some(Ok(ExitStatus {
            message: Some("exec: \"tar\": executable file not found in $PATH".into()),
            ..ExitStatus::default()
        })),
        ..Plan::exits(0)
    });
    assert_eq!(by_message.kind(), ErrorKind::Unsupported);

    // The runtime may refuse the exec itself.
    let port = Scripted::default();
    *port.open_error.lock() = Some(OxiError::conflict(
        "OCI runtime exec failed: exec: \"tar\": executable file not found in $PATH",
    ));
    let error = block_on(port.tar_download(&source(), TransferOptions::default()))
        .err()
        .expect("open fails");
    assert_eq!(error.kind(), ErrorKind::Unsupported);
}

#[test]
fn cancel_ends_the_stream_with_an_error_and_aborts_the_session() {
    let stdin = Stdin::new();
    let port = Scripted::with(Plan {
        stdin: Some(stdin.sink()),
        ..Plan::exits(0)
            .running()
            .stdout_chunks(vec![b"one".to_vec()])
            .chain_pending()
    });
    let cancel = CancellationToken::new();
    let options = TransferOptions {
        cancel: cancel.clone(),
        ..TransferOptions::default()
    };
    let mut stream = block_on(port.tar_download(&source(), options)).expect("open");
    let first = block_on(stream.next()).expect("a chunk").expect("ok");
    assert_eq!(&first[..], b"one");
    assert!(
        stream.next().now_or_never().is_none(),
        "waiting for more output"
    );
    assert!(!port.seen.aborted());
    cancel.cancel();
    let error = block_on(stream.next())
        .expect("an item")
        .expect_err("cancelled");
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert!(
        port.seen.aborted(),
        "the session's status was dropped unresolved"
    );
    assert!(block_on(stream.next()).is_none());
}

#[test]
fn dropping_the_stream_aborts_the_session() {
    let port = Scripted::with(Plan::exits(0).running().chain_pending());
    let stream = block_on(port.tar_download(&source(), TransferOptions::default())).expect("open");
    assert!(!port.seen.aborted());
    drop(stream);
    assert!(port.seen.aborted());
}

#[test]
fn a_cancelled_token_opens_nothing() {
    let port = Scripted::with(Plan::exits(0));
    let cancel = CancellationToken::new();
    cancel.cancel();
    let options = TransferOptions {
        cancel,
        ..TransferOptions::default()
    };
    let error = block_on(port.tar_download(&source(), options))
        .err()
        .expect("refused");
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert!(port.seen.command.lock().is_empty());
}

/// 200 MB through the stream, produced lazily: the stream never holds more than the chunk the
/// consumer is looking at.
#[test]
fn two_hundred_megabytes_stream_with_bounded_buffering() {
    const CHUNK: usize = 64 * 1024;
    const TOTAL: u64 = 200 * 1024 * 1024;
    let produced = Arc::new(AtomicU64::new(0));
    let counter = produced.clone();
    let stdout = stream::unfold(0u64, move |sent| {
        let counter = counter.clone();
        async move {
            if sent >= TOTAL {
                return None;
            }
            counter.fetch_add(CHUNK as u64, Ordering::SeqCst);
            Some((Ok(vec![7u8; CHUNK]), sent + CHUNK as u64))
        }
    })
    .boxed();
    let port = Scripted::with(Plan {
        stdout,
        ..Plan::exits(0)
    });
    let reports = Arc::new(Mutex::new(Vec::<TransferProgress>::new()));
    let sink = reports.clone();
    let options = TransferOptions {
        total_hint: Some(TOTAL),
        progress: Some(Arc::new(move |progress| sink.lock().push(progress))),
        ..TransferOptions::default()
    };
    let mut consumed = 0u64;
    let mut most_ahead = 0u64;
    block_on(async {
        let mut stream = port.tar_download(&source(), options).await.expect("open");
        while let Some(chunk) = stream.next().await {
            consumed += chunk.expect("chunk").len() as u64;
            most_ahead = most_ahead.max(produced.load(Ordering::SeqCst) - consumed);
        }
    });
    assert_eq!(consumed, TOTAL);
    assert_eq!(most_ahead, 0, "nothing is read ahead of the consumer");
    let reports = reports.lock();
    assert_eq!(reports.last().map(|p| p.bytes), Some(TOTAL));
    assert_eq!(reports.last().and_then(|p| p.total), Some(TOTAL));
}
