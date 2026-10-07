//! The byte-stream contract the terminal element drives: [`TerminalBackend`].

use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use oxikube_domain::{OxiError, OxiResult};

/// Terminal dimensions in character cells, with an optional pixel size. Mirrors kube's
/// `TerminalSize { width, height }`; the pixel size is for `TIOCSWINSZ` on a local PTY and
/// is ignored by the Kubernetes streaming protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TerminalSize {
    /// Columns.
    pub width: u16,
    /// Rows.
    pub height: u16,
    /// Width in pixels; `0` when unknown.
    pub pixel_width: u16,
    /// Height in pixels; `0` when unknown.
    pub pixel_height: u16,
}

impl TerminalSize {
    /// A size of `width` columns by `height` rows, pixel size unknown.
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            pixel_width: 0,
            pixel_height: 0,
        }
    }

    /// The same size with a pixel size attached.
    #[must_use]
    pub fn with_pixels(mut self, pixel_width: u16, pixel_height: u16) -> Self {
        self.pixel_width = pixel_width;
        self.pixel_height = pixel_height;
        self
    }
}

/// How the remote or local process ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExitStatus {
    /// Exit code when one was reported (`0` on success).
    pub code: Option<i32>,
    /// The terminating signal's name (`"KILL"`, `"HUP"`), when the process died to one.
    pub signal: Option<String>,
    /// A failure message (the server's status message, a spawn error), if any.
    pub message: Option<String>,
}

impl ExitStatus {
    /// A successful exit (code `0`).
    pub fn success() -> Self {
        Self {
            code: Some(0),
            signal: None,
            message: None,
        }
    }

    /// An exit with `code`.
    pub fn with_code(code: i32) -> Self {
        Self {
            code: Some(code),
            signal: None,
            message: None,
        }
    }

    /// A process ended by `signal`.
    pub fn killed_by(signal: impl Into<String>) -> Self {
        Self {
            code: None,
            signal: Some(signal.into()),
            message: None,
        }
    }

    /// Whether the process exited with code `0`.
    pub fn is_success(&self) -> bool {
        self.code == Some(0) && self.signal.is_none()
    }
}

/// One thing a [`TerminalBackend`] tells the terminal.
#[derive(Debug)]
pub enum BackendEvent {
    /// Bytes the process wrote (stdout, with a TTY the merged terminal output). Never
    /// persist or log them: they can carry secrets.
    Output(Bytes),
    /// The process ended. It is the last event of the stream.
    Exited(ExitStatus),
    /// The transport failed (connection dropped, read error). The stream may end after it;
    /// an exit status follows when one is still known.
    Error(OxiError),
}

/// A terminal session's byte pipe, the same for a local shell, a pod exec/attach, a node
/// shell and a debug container, so the grid, the element and the tab never know which one
/// they drive.
///
/// All methods take `&self` (a backend is shared as `Arc` / `Box` and driven from the
/// element and from background tasks); implementations synchronise internally. The trait is
/// object-safe: [`ExecPort`](super::ExecPort) hands out `Box<dyn TerminalBackend>`.
///
/// # Output and backpressure
///
/// [`output_stream`](Self::output_stream) is a **pull** stream: an implementation produces
/// the next chunk only when the consumer polls, or buffers in a *bounded* queue that stops
/// reading from the source when full. A flood (`yes`) therefore slows the producer instead
/// of growing memory, and the terminal can coalesce chunks to frame cadence. Chunks are
/// [`Bytes`], passed through without a per-byte copy.
///
/// The stream is **single-consumer**: the first call returns the live stream, later calls
/// return an empty (already ended) stream. It ends after [`BackendEvent::Exited`], and ends
/// when the backend is killed or dropped.
///
/// # Effects
///
/// [`write`](Self::write) sends keystrokes to a process the user controls; it is not a
/// `MutationGuard` operation (the guarded step is opening a node shell or a debug
/// container, see [`ExecPort`](super::ExecPort)). Nothing written or read is ever logged.
#[async_trait]
pub trait TerminalBackend: Send + Sync {
    /// Sends `bytes` to the process's stdin. Returns once they were handed to the
    /// transport; applies backpressure when the process is not reading.
    ///
    /// # Errors
    ///
    /// `Network` when the session is gone, `Conflict` after [`kill`](Self::kill) or after
    /// the process exited.
    async fn write(&self, bytes: &[u8]) -> OxiResult<()>;

    /// Tells the process its terminal is now `size`. A no-op for a session without a TTY.
    ///
    /// # Errors
    ///
    /// As [`write`](Self::write).
    async fn resize(&self, size: TerminalSize) -> OxiResult<()>;

    /// The events of the session: output, then exit. See the trait docs for the
    /// single-consumer and backpressure rules.
    fn output_stream(&self) -> BoxStream<'static, BackendEvent>;

    /// Ends the session now: closes stdin, tears down the transport (and, for a local PTY,
    /// the child). Idempotent: killing a dead or already killed backend succeeds. The
    /// output stream then ends, with [`BackendEvent::Exited`] when no exit was seen yet.
    async fn kill(&self) -> OxiResult<()>;

    /// The directory the session's foreground process works in right now, when the backend
    /// can tell: a local PTY reports its shell's directory after a `cd` (or the directory of
    /// what the shell runs). `None` for a session that cannot say (a pod exec or attach) or
    /// that has ended. The default is `None`.
    ///
    /// A split or a saved terminal tab starts its new shell there. The lookup is a couple of
    /// non-blocking syscalls, cheap enough for the UI thread on a user action.
    fn working_directory(&self) -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_status() {
        assert!(ExitStatus::success().is_success());
        assert!(ExitStatus::with_code(0).is_success());
        let failed = ExitStatus {
            message: Some("command terminated with non-zero exit code".into()),
            ..ExitStatus::with_code(2)
        };
        assert!(!failed.is_success());
        assert!(!ExitStatus::default().is_success());
        let killed = ExitStatus::killed_by("KILL");
        assert!(!killed.is_success());
        assert_eq!(killed.signal.as_deref(), Some("KILL"));
    }

    #[test]
    fn terminal_size_keeps_the_pixel_size_apart() {
        let size = TerminalSize::new(80, 24);
        assert_eq!((size.pixel_width, size.pixel_height), (0, 0));
        let sized = size.with_pixels(640, 480);
        assert_eq!((sized.width, sized.height), (80, 24));
        assert_eq!((sized.pixel_width, sized.pixel_height), (640, 480));
        assert_ne!(size, sized);
    }
}
