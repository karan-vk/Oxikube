//! The remote stdin as a `Sink` of byte chunks.

use std::pin::Pin;
use std::task::{Context, Poll, ready};

use futures::Sink;
use oxikube_domain::{OxiError, OxiResult};
use tokio::io::AsyncWrite;

/// Writes each chunk it is sent to `W`, one chunk in flight at a time: `poll_ready` returns
/// `Pending` while the previous chunk is not fully written, which is the backpressure (the
/// pipe behind `W` is bounded). Closing the sink shuts `W` down, which kube turns into an
/// end-of-input for the remote process.
pub(super) struct StdinWriter<W> {
    writer: Option<W>,
    pending: Vec<u8>,
    written: usize,
}

impl<W> StdinWriter<W> {
    pub(super) fn new(writer: W) -> Self {
        Self {
            writer: Some(writer),
            pending: Vec::new(),
            written: 0,
        }
    }
}

fn closed() -> OxiError {
    OxiError::network("the remote process is no longer reading stdin")
}

impl<W: AsyncWrite + Unpin> StdinWriter<W> {
    /// Writes out the chunk in flight.
    fn poll_drain(&mut self, cx: &mut Context<'_>) -> Poll<OxiResult<()>> {
        while self.written < self.pending.len() {
            let Some(writer) = self.writer.as_mut() else {
                return Poll::Ready(Err(closed()));
            };
            match ready!(Pin::new(writer).poll_write(cx, &self.pending[self.written..])) {
                Ok(0) | Err(_) => return Poll::Ready(Err(closed())),
                Ok(n) => self.written += n,
            }
        }
        self.pending.clear();
        self.written = 0;
        Poll::Ready(Ok(()))
    }
}

impl<W: AsyncWrite + Unpin> Sink<Vec<u8>> for StdinWriter<W> {
    type Error = OxiError;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<OxiResult<()>> {
        self.get_mut().poll_drain(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Vec<u8>) -> OxiResult<()> {
        let this = self.get_mut();
        if this.writer.is_none() {
            return Err(closed());
        }
        this.pending = item;
        this.written = 0;
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<OxiResult<()>> {
        let this = self.get_mut();
        ready!(this.poll_drain(cx))?;
        match this.writer.as_mut() {
            Some(writer) => Pin::new(writer).poll_flush(cx).map_err(|_| closed()),
            None => Poll::Ready(Ok(())),
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<OxiResult<()>> {
        let this = self.get_mut();
        if this.writer.is_none() {
            return Poll::Ready(Ok(()));
        }
        ready!(this.poll_drain(cx))?;
        if let Some(writer) = this.writer.as_mut() {
            // A pipe whose reader is already gone has nothing left to end.
            let _ = ready!(Pin::new(writer).poll_shutdown(cx));
        }
        this.writer = None;
        Poll::Ready(Ok(()))
    }
}
