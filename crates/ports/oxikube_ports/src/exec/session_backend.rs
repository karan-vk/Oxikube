//! [`SessionBackend`]: an [`ExecSession`] as a [`TerminalBackend`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use futures::SinkExt;
use futures::StreamExt;
use futures::channel::oneshot;
use futures::lock::Mutex as AsyncMutex;
use futures::stream::{self, BoxStream};
use oxikube_domain::{OxiError, OxiResult};
use parking_lot::Mutex;

use super::stream::{OutputStream, ResizeSink, StdinSink};
use super::{BackendEvent, ExecSession, ExitStatus, TerminalBackend, TerminalSize};

/// Adapts a stream-level [`ExecSession`] to the [`TerminalBackend`] contract without
/// spawning anything: output is pulled through the session's own bounded pipes, so a
/// consumer that stops polling stops the remote process (the backpressure rule of the
/// trait), and the exit status is read when the output ends.
///
/// Stdout and stderr (when both exist) are merged in arrival order. [`kill`] drops stdin
/// and the resize channel and ends the event stream with `Exited` (signal `KILL`); dropping
/// the session's `status` future is what closes the connection, so the stream's end (or
/// dropping the backend) releases it.
///
/// [`kill`]: TerminalBackend::kill
pub struct SessionBackend {
    stdin: AsyncMutex<Option<StdinSink>>,
    resize: AsyncMutex<Option<ResizeSink>>,
    events: Mutex<Option<BoxStream<'static, BackendEvent>>>,
    kill_tx: Mutex<Option<oneshot::Sender<()>>>,
    killed: Arc<AtomicBool>,
}

impl std::fmt::Debug for SessionBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionBackend")
            .field("killed", &self.killed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

fn into_events(stream: OutputStream) -> BoxStream<'static, BackendEvent> {
    stream
        .map(|chunk| match chunk {
            Ok(bytes) => BackendEvent::Output(Bytes::from(bytes)),
            Err(error) => BackendEvent::Error(error),
        })
        .boxed()
}

impl SessionBackend {
    /// Wraps `session`.
    pub fn new(session: ExecSession) -> Self {
        let ExecSession {
            stdin,
            stdout,
            stderr,
            resize,
            status,
        } = session;
        let killed = Arc::new(AtomicBool::new(false));
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let outputs = [stdout, stderr]
            .into_iter()
            .flatten()
            .map(into_events)
            .collect::<Vec<_>>();
        let data = stream::select_all(outputs).take_until(kill_rx);
        let was_killed = killed.clone();
        let last = stream::once(async move {
            if was_killed.load(Ordering::Acquire) {
                // `status` is dropped unawaited: that closes the connection.
                return BackendEvent::Exited(ExitStatus::killed_by("KILL"));
            }
            match status.await {
                Ok(exit) => BackendEvent::Exited(exit),
                Err(error) => BackendEvent::Error(error),
            }
        });
        Self {
            stdin: AsyncMutex::new(stdin),
            resize: AsyncMutex::new(resize),
            events: Mutex::new(Some(data.chain(last).boxed())),
            kill_tx: Mutex::new(Some(kill_tx)),
            killed,
        }
    }

    fn check_alive(&self) -> OxiResult<()> {
        if self.killed.load(Ordering::Acquire) {
            return Err(OxiError::conflict("the terminal session was closed"));
        }
        Ok(())
    }

    fn mark_killed(&self) {
        self.killed.store(true, Ordering::Release);
        if let Some(tx) = self.kill_tx.lock().take() {
            // The receiver may be gone already (stream taken and dropped): that is fine.
            let _ = tx.send(());
        }
    }
}

impl Drop for SessionBackend {
    fn drop(&mut self) {
        self.mark_killed();
    }
}

#[async_trait]
impl TerminalBackend for SessionBackend {
    async fn write(&self, bytes: &[u8]) -> OxiResult<()> {
        self.check_alive()?;
        let mut stdin = self.stdin.lock().await;
        match stdin.as_mut() {
            Some(sink) => sink.send(bytes.to_vec()).await,
            None => Err(OxiError::unsupported("this session has no stdin")),
        }
    }

    async fn resize(&self, size: TerminalSize) -> OxiResult<()> {
        self.check_alive()?;
        match self.resize.lock().await.as_mut() {
            Some(sink) => sink.send(size).await,
            // No TTY: there is nothing to resize.
            None => Ok(()),
        }
    }

    fn output_stream(&self) -> BoxStream<'static, BackendEvent> {
        self.events
            .lock()
            .take()
            .unwrap_or_else(|| stream::empty().boxed())
    }

    async fn kill(&self) -> OxiResult<()> {
        self.mark_killed();
        // Unconsumed events own the status future: dropping them closes the connection.
        drop(self.events.lock().take());
        // A write blocked on backpressure holds the lock; it fails once the connection
        // closes, so skip the sink instead of waiting for it.
        if let Some(mut stdin) = self.stdin.try_lock() {
            drop(stdin.take());
        }
        if let Some(mut resize) = self.resize.try_lock() {
            drop(resize.take());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
