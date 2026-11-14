//! A guest's stdout or stderr, written to `tracing` with secrets redacted.

use oxikube_domain::redact::redact;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use tokio::io::AsyncWrite;
use wasmtime_wasi::cli::IsTerminal;
use wasmtime_wasi::cli::StdoutStream;

/// One output stream of one extension. Each write becomes one `tracing` event.
#[derive(Clone)]
pub(super) struct LogStream {
    extension: Arc<str>,
    stream: &'static str,
}

impl LogStream {
    /// The stream `stream` ("stdout" or "stderr") of the extension `extension`.
    pub(super) fn new(extension: &str, stream: &'static str) -> Self {
        Self {
            extension: extension.into(),
            stream,
        }
    }
}

impl IsTerminal for LogStream {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for LogStream {
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

impl AsyncWrite for LogStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let text = String::from_utf8_lossy(buf);
        tracing::info!(
            target: "oxikube::extension",
            extension = %self.extension,
            stream = self.stream,
            "{}",
            redact(text.trim_end())
        );
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
