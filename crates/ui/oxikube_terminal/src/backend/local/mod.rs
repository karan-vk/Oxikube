//! [`LocalPty`]: the user's shell on a pseudo-terminal, as a
//! [`TerminalBackend`] (E09-S02).
//!
//! The grid, the element and the tab drive it exactly like a pod exec session. For a cluster
//! tab the shell starts with `KUBECONFIG` (a private merged file), `KUBE_CONTEXT` and
//! `OXIKUBE_NAMESPACE` set, so `kubectl`, `helm` and `argocd` target the tab's cluster; `PATH`
//! is inherited untouched.
//!
//! # Threads
//!
//! [`LocalPty::spawn`] is synchronous (it forks): call it from a background task, never from
//! the UI thread. Afterwards three OS threads serve the session (read, write, wait for the
//! child) and all of them end with the backend: dropping a [`LocalPty`] kills the child's
//! process group, which closes the PTY, which ends the reader; the writer ends when its queue
//! closes. Nothing here is a GPUI task.
//!
//! # Privacy
//!
//! Output and input are never logged, stored or put in an error message. The merged kubeconfig
//! is deleted when the shell exits or the backend is dropped.

mod io;
mod kubeconfig;
mod options;
mod unix;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::channel::oneshot;
use futures::stream::{self, BoxStream};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::exec::{BackendEvent, TerminalBackend, TerminalSize};
use parking_lot::Mutex;
use portable_pty::{MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc;

use self::io::{ReaderState, WriteRequest};
pub use kubeconfig::{
    ClusterEnv, PreparedEnv, TempKubeconfig, cleanup_runtime_dir, files_of_sources,
};
pub use options::{DEFAULT_SIZE, LocalPtyOptions, resolve_shell};

/// Chunks the output queue holds before the reader stops reading from the PTY
/// (about 512 KiB): a flood slows the shell down instead of growing memory.
const OUTPUT_QUEUE: usize = 32;
/// Writes queued ahead of the PTY.
const WRITE_QUEUE: usize = 64;

/// State the three threads and the backend share.
struct Shared {
    reader: Arc<ReaderState>,
    /// [`kill`](TerminalBackend::kill) or drop was called.
    killed: AtomicBool,
    /// The shell has ended (and was reaped).
    exited: AtomicBool,
    /// The child's pid until it is reaped; nothing is signalled afterwards (pids get reused).
    pid: Mutex<Option<u32>>,
    /// The merged kubeconfig, removed when the shell ends.
    kubeconfig: Mutex<Option<TempKubeconfig>>,
}

impl Shared {
    fn kill_group(&self) {
        self.killed.store(true, Ordering::Release);
        if let Some(pid) = *self.pid.lock() {
            unix::kill_group(pid);
        }
    }
}

/// A local shell on a PTY. See the [module docs](self).
pub struct LocalPty {
    shared: Arc<Shared>,
    events: Mutex<Option<BoxStream<'static, BackendEvent>>>,
    writes: mpsc::Sender<WriteRequest>,
    master: Mutex<Box<dyn MasterPty + Send>>,
}

impl std::fmt::Debug for LocalPty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalPty")
            .field("pid", &*self.shared.pid.lock())
            .field("exited", &self.shared.exited.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

fn pty_size(size: TerminalSize) -> PtySize {
    PtySize {
        rows: size.height,
        cols: size.width,
        pixel_width: size.pixel_width,
        pixel_height: size.pixel_height,
    }
}

fn internal(what: &str, error: impl std::fmt::Display) -> OxiError {
    OxiError::internal(format!("{what}: {error}"))
}

impl LocalPty {
    /// Starts the shell `options` describe.
    ///
    /// Blocking (reads kubeconfig files, forks): run it off the UI thread.
    ///
    /// # Errors
    ///
    /// Those of [`ClusterEnv::prepare`] for a cluster terminal, `Validation` when the shell
    /// cannot be started (missing, not executable) or the size is zero, `Internal` when the
    /// PTY cannot be opened.
    pub fn spawn(options: LocalPtyOptions) -> OxiResult<Self> {
        validate_size(options.size)?;
        // Dropping `prepared` on any early return below deletes the merged file.
        let prepared = options
            .cluster
            .as_ref()
            .map(ClusterEnv::prepare)
            .transpose()?;
        let vars = prepared.as_ref().map_or(&[][..], |p| p.vars.as_slice());
        let command = options::build_command(&options, vars);

        let pair = native_pty_system()
            .openpty(pty_size(options.size))
            .map_err(|e| internal("could not open a terminal", e))?;
        let mut child = pair.slave.spawn_command(command).map_err(|e| {
            OxiError::validation(format!(
                "could not start the shell `{}`: {e}",
                options.resolved_shell()
            ))
        })?;
        // The child holds the only slave end now; keeping ours would stop the reader seeing
        // the end of the output.
        drop(pair.slave);
        // A failure from here on must not leave the shell running with nobody attached.
        let ends = (|| {
            let pid = child
                .process_id()
                .ok_or_else(|| OxiError::internal("the shell has no process id"))?;
            let reader = pair
                .master
                .try_clone_reader()
                .map_err(|e| internal("could not read from the terminal", e))?;
            let writer = pair
                .master
                .take_writer()
                .map_err(|e| internal("could not write to the terminal", e))?;
            Ok((pid, reader, writer))
        })();
        let (pid, reader, writer) = match ends {
            Ok(ends) => ends,
            Err(error) => {
                let _ = child.kill();
                return Err(error);
            }
        };

        let shared = Arc::new(Shared {
            reader: Arc::new(ReaderState::default()),
            killed: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            pid: Mutex::new(Some(pid)),
            kubeconfig: Mutex::new(prepared.map(|p| p.file)),
        });
        let (event_tx, event_rx) = mpsc::channel(OUTPUT_QUEUE);
        let (write_tx, write_rx) = mpsc::channel(WRITE_QUEUE);

        let started = io::spawn_reader(reader, event_tx.clone(), shared.reader.clone())
            .and_then(|()| io::spawn_writer(writer, write_rx))
            .and_then(|()| {
                let shared = shared.clone();
                std::thread::Builder::new()
                    .name("oxikube-pty-wait".into())
                    .spawn(move || wait_for_child(&shared, &mut *child, pid, &event_tx))
                    .map(drop)
            });
        if let Err(e) = started {
            shared.kill_group();
            return Err(internal("could not start the terminal threads", e));
        }

        Ok(Self {
            shared,
            events: Mutex::new(Some(into_stream(event_rx))),
            writes: write_tx,
            master: Mutex::new(pair.master),
        })
    }

    /// The shell's process id, until it ends.
    pub fn process_id(&self) -> Option<u32> {
        *self.shared.pid.lock()
    }

    fn check_alive(&self) -> OxiResult<()> {
        if self.shared.killed.load(Ordering::Acquire) {
            return Err(OxiError::conflict("the terminal session was closed"));
        }
        if self.shared.exited.load(Ordering::Acquire) {
            return Err(OxiError::conflict("the shell has exited"));
        }
        Ok(())
    }
}

/// Runs on the waiter thread: reaps the child, removes the kubeconfig, lets the output drain
/// and reports the exit as the last event.
fn wait_for_child(
    shared: &Shared,
    child: &mut (dyn portable_pty::Child + Send + Sync),
    pid: u32,
    events: &mpsc::Sender<BackendEvent>,
) {
    let status = unix::wait_for_exit(child, pid);
    *shared.pid.lock() = None;
    shared.exited.store(true, Ordering::Release);
    drop(shared.kubeconfig.lock().take());
    io::wait_for_output_end(&shared.reader);
    io::send(events, BackendEvent::Exited(status), &shared.reader.stop);
    // Nothing more is delivered, even when a leftover process still writes to the PTY.
    shared.reader.stop.store(true, Ordering::Release);
}

/// The event queue as a stream that ends right after the exit, whatever else holds a sender.
fn into_stream(rx: mpsc::Receiver<BackendEvent>) -> BoxStream<'static, BackendEvent> {
    stream::unfold((rx, false), |(mut rx, finished)| async move {
        if finished {
            return None;
        }
        let event = rx.recv().await?;
        let last = matches!(event, BackendEvent::Exited(_));
        Some((event, (rx, last)))
    })
    .boxed()
}

fn validate_size(size: TerminalSize) -> OxiResult<()> {
    if size.width == 0 || size.height == 0 {
        return Err(OxiError::validation(
            "a terminal needs at least 1 column and 1 row",
        ));
    }
    Ok(())
}

impl Drop for LocalPty {
    fn drop(&mut self) {
        self.shared.reader.stop.store(true, Ordering::Release);
        self.shared.kill_group();
        // Do not wait for the waiter thread: the file is gone when the backend is.
        drop(self.shared.kubeconfig.lock().take());
    }
}

#[async_trait]
impl TerminalBackend for LocalPty {
    async fn write(&self, bytes: &[u8]) -> OxiResult<()> {
        self.check_alive()?;
        let (ack, done) = oneshot::channel();
        let request = WriteRequest {
            bytes: bytes.to_vec(),
            ack,
        };
        let closed = || OxiError::conflict("the terminal session was closed");
        // A full queue (the shell is not reading) makes this wait: that is the backpressure.
        self.writes.send(request).await.map_err(|_| closed())?;
        done.await.map_err(|_| closed())?
    }

    async fn resize(&self, size: TerminalSize) -> OxiResult<()> {
        self.check_alive()?;
        validate_size(size)?;
        // One `ioctl`: it does not block.
        self.master
            .lock()
            .resize(pty_size(size))
            .map_err(|e| internal("could not resize the terminal", e))
    }

    fn output_stream(&self) -> BoxStream<'static, BackendEvent> {
        self.events
            .lock()
            .take()
            .unwrap_or_else(|| stream::empty().boxed())
    }

    async fn kill(&self) -> OxiResult<()> {
        self.shared.kill_group();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
