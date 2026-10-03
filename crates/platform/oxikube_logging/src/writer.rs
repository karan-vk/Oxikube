//! A `MakeWriter` / `io::Write` pair that scrubs everything written through it.

use oxikube_domain::redact::redact;
use std::io;
use tracing::Metadata;
use tracing_subscriber::fmt::MakeWriter;

/// Wraps a [`MakeWriter`] so every writer it hands out is a [`RedactingWriter`].
#[derive(Debug, Clone, Default)]
pub struct RedactingMakeWriter<M> {
    inner: M,
}

impl<M> RedactingMakeWriter<M> {
    /// Wraps `inner`.
    pub fn new(inner: M) -> Self {
        Self { inner }
    }
}

impl<'a, M> MakeWriter<'a> for RedactingMakeWriter<M>
where
    M: MakeWriter<'a>,
{
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(self.inner.make_writer())
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        RedactingWriter::new(self.inner.make_writer_for(meta))
    }
}

/// Scrubs each `write` call with [`redact`] before passing it to the inner writer.
///
/// Each call is treated as a whole unit of text (for tracing, one formatted event). Invalid
/// UTF-8 is replaced with U+FFFD before scrubbing: a log line is text, and un-scannable bytes
/// must not be a way around the scrubber.
#[derive(Debug)]
pub struct RedactingWriter<W> {
    inner: W,
}

impl<W> RedactingWriter<W> {
    /// Wraps `inner`.
    pub fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<W: io::Write> io::Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match std::str::from_utf8(buf) {
            Ok(text) => self.inner.write_all(redact(text).as_bytes())?,
            Err(_) => {
                let text = String::from_utf8_lossy(buf);
                self.inner.write_all(redact(&text).as_bytes())?;
            }
        }
        // Report the caller's byte count, not the (possibly different) scrubbed length.
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
