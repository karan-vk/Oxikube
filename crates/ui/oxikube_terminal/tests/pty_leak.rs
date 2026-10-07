//! 50 open/close cycles of a real `LocalPty` leave neither tasks nor file descriptors behind
//! (E09-S12, the PTY-gated variant of the leak test in `tests/view/leak.rs`). Unix only: it counts
//! the process's open descriptors through `/dev/fd`.
#![cfg(unix)]

use std::time::Duration;

use futures::StreamExt as _;
use oxikube_ports::exec::{BackendEvent, TerminalBackend as _};
use oxikube_terminal::backend::local::{LocalPty, LocalPtyOptions};

fn open_fds() -> usize {
    std::fs::read_dir("/dev/fd").map_or(0, |entries| entries.count())
}

fn sh(script: &str) -> LocalPty {
    LocalPty::spawn(LocalPtyOptions {
        shell: Some("/bin/sh".into()),
        args: vec!["-c".into(), script.into()],
        ..LocalPtyOptions::default()
    })
    .expect("spawn /bin/sh")
}

/// Waits (briefly) for `count` to drop to `target`: closing is asynchronous (a reader thread and
/// an abort land after the call returns).
async fn settles_to(target: usize, count: impl Fn() -> usize) -> usize {
    for _ in 0..200 {
        if count() <= target {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fifty_pty_open_close_cycles_leak_no_task_and_no_descriptor() {
    let metrics = tokio::runtime::Handle::current().metrics();
    // One warm-up cycle: lazily opened descriptors (the runtime's, the PTY master device) do not
    // count as growth.
    {
        let pty = sh("sleep 60");
        drop(pty.output_stream());
        pty.kill().await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let fds_before = open_fds();
    let tasks_before = metrics.num_alive_tasks();

    for cycle in 0..50 {
        let pty = sh("echo ready; sleep 60");
        let mut events = pty.output_stream();
        // A reader like the terminal's pump: it ends when the stream does.
        let pump = tokio::spawn(async move {
            while let Some(event) = events.next().await {
                if matches!(event, BackendEvent::Exited(_)) {
                    break;
                }
            }
        });
        pty.write(b"x").await.unwrap();
        // Closing the tab: kill the process, abort the reader, drop the backend.
        pty.kill().await.unwrap();
        pump.abort();
        let _ = pump.await;
        drop(pty);
        if cycle % 10 == 9 {
            tokio::task::yield_now().await;
        }
    }

    let fds_after = settles_to(fds_before, open_fds).await;
    let tasks_after = settles_to(tasks_before, || metrics.num_alive_tasks()).await;
    eprintln!(
        "pty leak test: open descriptors {fds_before} -> {fds_after}, tokio tasks {tasks_before} -> {tasks_after}"
    );
    assert!(
        fds_after <= fds_before,
        "descriptors grew: {fds_before} -> {fds_after}"
    );
    assert!(
        tasks_after <= tasks_before,
        "tasks grew: {tasks_before} -> {tasks_after}"
    );
}
