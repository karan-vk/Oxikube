//! [`NoticeBackend`]: a backend whose output starts with one line from Oxikube.

use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt as _;
use futures::stream::{self, BoxStream};
use oxikube_domain::OxiResult;
use oxikube_ports::{BackendEvent, TerminalBackend, TerminalSize};

/// The line a terminal shows before the process's own output: which shell opened, in which
/// container, or why another one did. Dim, one line, so it reads as the app's and not the
/// container's.
///
/// `text` is cleaned of control characters first: a shell path from the settings must not be
/// able to send escape sequences.
pub(super) fn notice_line(text: &str) -> Bytes {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    Bytes::from(format!("\x1b[2m[oxikube] {clean}\x1b[0m\r\n"))
}

/// A [`TerminalBackend`] that hands out `notice` as the first output event and then whatever
/// the wrapped backend sends. Everything else is the wrapped backend's.
pub(super) struct NoticeBackend {
    inner: Box<dyn TerminalBackend>,
    notice: Bytes,
}

impl NoticeBackend {
    /// `inner`, with `notice` shown first.
    pub(super) fn new(inner: Box<dyn TerminalBackend>, notice: Bytes) -> Self {
        Self { inner, notice }
    }
}

#[async_trait]
impl TerminalBackend for NoticeBackend {
    async fn write(&self, bytes: &[u8]) -> OxiResult<()> {
        self.inner.write(bytes).await
    }

    async fn resize(&self, size: TerminalSize) -> OxiResult<()> {
        self.inner.resize(size).await
    }

    fn output_stream(&self) -> BoxStream<'static, BackendEvent> {
        let notice = BackendEvent::Output(self.notice.clone());
        stream::once(async move { notice })
            .chain(self.inner.output_stream())
            .boxed()
    }

    async fn kill(&self) -> OxiResult<()> {
        self.inner.kill().await
    }

    fn working_directory(&self) -> Option<PathBuf> {
        self.inner.working_directory()
    }
}
