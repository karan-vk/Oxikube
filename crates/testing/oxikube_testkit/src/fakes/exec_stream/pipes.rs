//! The session pieces the test observes: stdin that records its close, a status that records
//! being dropped unresolved.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Sink;
use oxikube_domain::{OxiError, OxiResult};
use parking_lot::Mutex;

use super::ExecShared;

/// Owned by a session's `status`: marks the session aborted when the future is dropped before
/// it resolved.
pub(super) struct Ended {
    shared: Arc<Mutex<ExecShared>>,
    resolved: bool,
}

impl Ended {
    pub(super) fn new(shared: Arc<Mutex<ExecShared>>) -> Self {
        Self {
            shared,
            resolved: false,
        }
    }

    /// The status resolved: dropping it now is a normal end.
    pub(super) fn resolve(&mut self) {
        self.resolved = true;
    }
}

impl Drop for Ended {
    fn drop(&mut self) {
        if !self.resolved {
            self.shared.lock().aborted = true;
        }
    }
}

/// The session's stdin: records (or counts) what it is sent, and its close.
pub(super) struct CaptureStdin {
    shared: Arc<Mutex<ExecShared>>,
}

impl CaptureStdin {
    pub(super) fn new(shared: Arc<Mutex<ExecShared>>) -> Self {
        Self { shared }
    }
}

impl Sink<Vec<u8>> for CaptureStdin {
    type Error = OxiError;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, bytes: Vec<u8>) -> OxiResult<()> {
        let mut shared = self.shared.lock();
        if shared.stdin_closed {
            return Err(OxiError::network("stdin is closed"));
        }
        shared.stdin_len += bytes.len() as u64;
        if !shared.discard_stdin {
            shared.stdin.extend_from_slice(&bytes);
        }
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        self.shared.lock().stdin_closed = true;
        Poll::Ready(Ok(()))
    }
}
