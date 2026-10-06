//! The threads behind a local PTY: one reads, one writes, one waits for the child. They are
//! plain OS threads because the PTY file descriptors are blocking; they talk to the async side
//! through bounded channels and never touch GPUI.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::channel::oneshot;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::exec::BackendEvent;
use tokio::sync::mpsc::{self, error::TrySendError};

/// Bytes read from the PTY per call. Large enough that `yes` costs few syscalls, small enough
/// that one chunk never holds the grid for long.
const READ_SIZE: usize = 16 * 1024;

/// How long the output may stay silent after the shell exited before the exit is reported
/// anyway. Only a background process that keeps the PTY open (`sleep 100 &` then `exit`)
/// reaches it.
const SILENCE_AFTER_EXIT: Duration = Duration::from_millis(150);

/// What the reader thread tells the waiter thread.
#[derive(Debug, Default)]
pub(super) struct ReaderState {
    /// The reader is inside `read`, not delivering a chunk.
    in_read: AtomicBool,
    /// Counts finished reads.
    activity: AtomicU64,
    /// The reader reached the end of the output.
    done: AtomicBool,
    /// The backend was dropped or the exit was reported: stop delivering.
    pub(super) stop: AtomicBool,
}

/// One write to the child's stdin, acknowledged when it reached the PTY.
pub(super) struct WriteRequest {
    pub(super) bytes: Vec<u8>,
    pub(super) ack: oneshot::Sender<OxiResult<()>>,
}

pub(super) fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::Sender<BackendEvent>,
    state: std::sync::Arc<ReaderState>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("oxikube-pty-read".into())
        .spawn(move || {
            let mut buf = vec![0u8; READ_SIZE];
            loop {
                state.in_read.store(true, Ordering::Release);
                let read = reader.read(&mut buf);
                state.in_read.store(false, Ordering::Release);
                state.activity.fetch_add(1, Ordering::Release);
                match read {
                    // Linux reports the closed slave as EIO, macOS as end of file.
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = BackendEvent::Output(Bytes::copy_from_slice(&buf[..n]));
                        if !send(&tx, chunk, &state.stop) {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) if is_closed_pty(&e) => break,
                    Err(e) => {
                        let error = OxiError::network(format!("terminal read failed: {e}"));
                        send(&tx, BackendEvent::Error(error), &state.stop);
                        break;
                    }
                }
            }
            state.done.store(true, Ordering::Release);
        })
        .map(drop)
}

pub(super) fn spawn_writer(
    mut writer: Box<dyn Write + Send>,
    mut rx: mpsc::Receiver<WriteRequest>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("oxikube-pty-write".into())
        .spawn(move || {
            while let Some(request) = rx.blocking_recv() {
                let result = writer
                    .write_all(&request.bytes)
                    .and_then(|()| writer.flush())
                    .map_err(|e| OxiError::network(format!("terminal write failed: {e}")));
                // The caller may have stopped waiting; the bytes are written either way.
                let _ = request.ack.send(result);
            }
        })
        .map(drop)
}

/// Blocks until every byte the shell wrote is delivered, or the output has been silent for
/// [`SILENCE_AFTER_EXIT`] with the reader waiting in `read` (a leftover process holds the PTY).
/// A reader that is blocked delivering to a slow consumer is waited for, not given up on.
pub(super) fn wait_for_output_end(state: &ReaderState) {
    let mut seen = state.activity.load(Ordering::Acquire);
    let mut quiet_since = Instant::now();
    while !state.done.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(5));
        let now = state.activity.load(Ordering::Acquire);
        if now != seen || !state.in_read.load(Ordering::Acquire) {
            seen = now;
            quiet_since = Instant::now();
        } else if quiet_since.elapsed() >= SILENCE_AFTER_EXIT {
            return;
        }
    }
}

/// Queues `event` for the consumer without ever blocking on a full queue forever: a consumer
/// that stopped polling slows the shell down (backpressure), and dropping the backend
/// (`stop`) releases the thread. Returns whether the event was queued.
pub(super) fn send(
    tx: &mpsc::Sender<BackendEvent>,
    event: BackendEvent,
    stop: &AtomicBool,
) -> bool {
    let mut event = event;
    loop {
        match tx.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Closed(_)) => return false,
            Err(TrySendError::Full(back)) => {
                if stop.load(Ordering::Acquire) {
                    return false;
                }
                event = back;
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

fn is_closed_pty(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    if error.raw_os_error() == Some(libc::EIO) {
        return true;
    }
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof
    )
}
