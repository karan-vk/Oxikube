//! The scripted sessions: output, stdin, resizes, ends, aborts.

use futures::executor::block_on;
use futures::{FutureExt as _, SinkExt as _, StreamExt as _};
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{ExecOptions, ExecStreamPort, ExitStatus, TerminalSize};

use super::{ExecScript, ExecStreamCall, FakeExecStreamPort};

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

/// Sends every item of `items` into `sink`, then closes it.
async fn send_all<S, T>(sink: &mut S, items: Vec<T>) -> Result<(), S::Error>
where
    S: futures::Sink<T> + Unpin,
{
    for item in items {
        sink.send(item).await?;
    }
    sink.close().await
}

#[test]
fn exec_scripted_session_produces_output_and_captures_stdin() {
    let fake = FakeExecStreamPort::new();
    let script = ExecScript::new()
        .stdout("hello ")
        .stdout("world")
        .stderr("warn")
        .exit(Ok(ExitStatus::with_code(2)));
    let capture = script.capture();
    fake.script()
        .exec
        .push_ok(script)
        .push_err(OxiError::forbidden("no exec"));

    let options = ExecOptions {
        stdin: true,
        ..ExecOptions::default()
    };
    let mut session =
        block_on(fake.exec_session("demo", "web", &argv(&["sh", "-c", "true"]), &options)).unwrap();
    assert!(session.resize.is_none());
    let out: Vec<u8> = block_on(session.stdout.take().unwrap().map(Result::unwrap).concat());
    assert_eq!(out, b"hello world");
    let err: Vec<u8> = block_on(session.stderr.take().unwrap().map(Result::unwrap).concat());
    assert_eq!(err, b"warn");
    block_on(send_all(
        session.stdin.as_mut().unwrap(),
        vec![b"ls\n".to_vec(), b"exit\n".to_vec()],
    ))
    .unwrap();
    assert_eq!(capture.stdin(), b"ls\nexit\n");
    assert_eq!(block_on(session.status).unwrap().code, Some(2));

    let denied = block_on(fake.exec_session("demo", "web", &argv(&["id"]), &options)).unwrap_err();
    assert_eq!(denied.kind(), ErrorKind::Forbidden);
    assert_eq!(
        fake.recorded_calls()[0],
        ExecStreamCall::Exec {
            namespace: "demo".into(),
            pod: "web".into(),
            command: argv(&["sh", "-c", "true"]),
            options,
        }
    );
    assert_eq!(fake.recorded_calls().len(), 2);
}

#[test]
fn attach_with_tty_captures_resizes_and_waits_for_finish() {
    let fake = FakeExecStreamPort::new();
    let script = ExecScript::new().exit_when_told();
    let capture = script.capture();
    fake.script().attach.push_ok(script);
    let mut session =
        block_on(fake.attach_session("demo", "web", &ExecOptions::interactive())).unwrap();
    assert!(session.stderr.is_none(), "no stderr with a tty");
    block_on(send_all(
        session.resize.as_mut().unwrap(),
        vec![TerminalSize::new(80, 24), TerminalSize::new(120, 40)],
    ))
    .unwrap();
    assert_eq!(
        capture.resizes(),
        vec![TerminalSize::new(80, 24), TerminalSize::new(120, 40)]
    );
    let mut status = session.status;
    assert!((&mut status).now_or_never().is_none());
    assert!(capture.finish(Ok(ExitStatus::success())));
    assert!(block_on(status).unwrap().is_success());
    assert!(!capture.finish(Ok(ExitStatus::success())));
    assert!(matches!(
        fake.recorded_calls()[0],
        ExecStreamCall::Attach { .. }
    ));
    assert!(block_on(fake.attach_session("demo", "web", &ExecOptions::default())).is_err());
}

#[test]
fn stdin_close_abort_and_lazy_output_are_observable() {
    let fake = FakeExecStreamPort::new();
    let produced = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = produced.clone();
    let output = futures::stream::iter(0..3)
        .map(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(vec![1u8; 4])
        })
        .boxed();
    let script = ExecScript::new().stdout_stream(output).exit_when_told();
    let capture = script.capture();
    fake.script().exec.push_ok(script);
    let options = ExecOptions {
        stdin: true,
        ..ExecOptions::default()
    };
    let mut session =
        block_on(fake.exec_session("demo", "web", &argv(&["tar"]), &options)).unwrap();
    let mut stdout = session.stdout.take().unwrap();
    assert_eq!(
        produced.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "nothing read yet"
    );
    assert_eq!(block_on(stdout.next()).unwrap().unwrap().len(), 4);
    assert_eq!(produced.load(std::sync::atomic::Ordering::SeqCst), 1);
    let stdin = session.stdin.as_mut().unwrap();
    block_on(send_all(stdin, vec![b"tar".to_vec()])).unwrap();
    assert!(capture.stdin_closed());
    assert!(block_on(stdin.send(b"late".to_vec())).is_err());
    assert_eq!(capture.stdin_len(), 3);
    assert!(!capture.aborted());
    drop(session);
    assert!(capture.aborted(), "status dropped before the session ended");

    // A session that ended normally was not aborted; discarded stdin is only counted.
    let script = ExecScript::new().discard_stdin();
    let capture = script.capture();
    fake.script().exec.push_ok(script);
    let mut session =
        block_on(fake.exec_session("demo", "web", &argv(&["tar"]), &options)).unwrap();
    block_on(send_all(
        session.stdin.as_mut().unwrap(),
        vec![vec![0u8; 1000]],
    ))
    .unwrap();
    assert!(block_on(session.status).unwrap().is_success());
    assert!(!capture.aborted());
    assert_eq!(capture.stdin_len(), 1000);
    assert!(capture.stdin().is_empty());
}
