//! Bounded, cancel-safe line reading and the kubelet timestamp prefix.
//!
//! The server sends `<RFC 3339 nano timestamp> <text>\n` per line (timestamps are always
//! requested; the prefix is stripped before a [`LogLine`](oxikube_domain::log::LogLine) is
//! built whether or not the caller asked for it). [`LineReader`] reads those lines out of the
//! response with one reusable byte buffer and one `String` allocation per line, and never
//! holds more than [`MAX_RAW_LINE_BYTES`] of a single line: a line without a newline for
//! megabytes cannot grow memory (the rest is discarded and the line is flagged cut).

use std::hash::{Hash, Hasher};
use std::io;
use std::pin::Pin;
use std::str::FromStr;
use std::task::{Context, Poll, ready};

use jiff::Timestamp;
use oxikube_domain::log::MAX_LOG_LINE_BYTES;

use futures::io::AsyncBufRead;

use super::source::Reader;

/// Room for the timestamp prefix on top of the longest kept text.
const PREFIX_ALLOWANCE: usize = 64;

/// The most bytes of one raw line (prefix included) that are buffered.
pub(crate) const MAX_RAW_LINE_BYTES: usize = MAX_LOG_LINE_BYTES + PREFIX_ALLOWANCE;

/// One line off the wire.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Line {
    /// The kubelet timestamp, `None` when the line had no parsable prefix.
    pub(crate) ts: Option<Timestamp>,
    /// The text without prefix, newline or a trailing `\r`.
    pub(crate) text: String,
    /// Hash of the timestamp and text: the dedup key.
    pub(crate) key: u64,
    /// Whether the reader dropped the rest of an over-long line.
    pub(crate) cut: bool,
}

/// Reads [`Line`]s from a response.
pub(crate) struct LineReader {
    inner: Reader,
    buf: Vec<u8>,
    cut: bool,
}

impl LineReader {
    pub(crate) fn new(inner: Reader) -> Self {
        Self {
            inner,
            buf: Vec::with_capacity(1024),
            cut: false,
        }
    }

    /// The next line, `None` at the end of the response. Cancel-safe: partial input stays in
    /// the reader, so dropping the future (a flush timer firing) loses nothing.
    pub(crate) async fn next_line(&mut self) -> io::Result<Option<Line>> {
        std::future::poll_fn(|cx| self.poll_line(cx)).await
    }

    fn poll_line(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<Option<Line>>> {
        loop {
            let chunk = ready!(self.inner.as_mut().poll_fill_buf(cx))?;
            if chunk.is_empty() {
                // End of the response; a last line without a newline still counts.
                return Poll::Ready(Ok((!self.buf.is_empty()).then(|| self.take_line())));
            }
            let newline = chunk.iter().position(|b| *b == b'\n');
            let used = newline.unwrap_or(chunk.len());
            append(&mut self.buf, &mut self.cut, &chunk[..used]);
            let consumed = newline.map_or(used, |i| i + 1);
            Pin::new(&mut self.inner).consume(consumed);
            if newline.is_some() {
                return Poll::Ready(Ok(Some(self.take_line())));
            }
        }
    }

    fn take_line(&mut self) -> Line {
        let mut raw = self.buf.as_slice();
        if let [rest @ .., b'\r'] = raw {
            raw = rest;
        }
        let (ts, text) = split_timestamp(raw);
        let line = Line {
            ts,
            key: key(ts, text),
            text: String::from_utf8_lossy(text).into_owned(),
            cut: self.cut,
        };
        self.buf.clear();
        self.cut = false;
        line
    }
}

/// Appends up to the buffer cap; anything beyond is dropped and flagged.
fn append(buf: &mut Vec<u8>, cut: &mut bool, bytes: &[u8]) {
    let room = MAX_RAW_LINE_BYTES.saturating_sub(buf.len());
    if bytes.len() > room {
        *cut = true;
    }
    buf.extend_from_slice(&bytes[..bytes.len().min(room)]);
}

/// Splits `raw` into the kubelet timestamp and the text after it. A line without a valid
/// prefix is returned whole with no timestamp.
fn split_timestamp(raw: &[u8]) -> (Option<Timestamp>, &[u8]) {
    let head = &raw[..raw.len().min(40)];
    // The prefix is `YYYY-MM-DDThh:mm:ss[.nnnnnnnnn]Z`.
    if head.len() < 20 || !head[0].is_ascii_digit() {
        return (None, raw);
    }
    let (stamp, text) = match head.iter().position(|b| *b == b' ') {
        Some(space) => (&raw[..space], &raw[space + 1..]),
        None if raw.len() <= 40 => (raw, &raw[raw.len()..]),
        None => return (None, raw),
    };
    match std::str::from_utf8(stamp)
        .ok()
        .and_then(|s| Timestamp::from_str(s).ok())
    {
        Some(ts) => (Some(ts), text),
        None => (None, raw),
    }
}

/// The dedup key of a line: its timestamp (so repeated text at different times stays
/// distinct) and its text.
fn key(ts: Option<Timestamp>, text: &[u8]) -> u64 {
    // `DefaultHasher::new` uses fixed keys, so keys are stable within the process, which is all
    // dedup needs.
    let mut hasher = std::hash::DefaultHasher::new();
    ts.map(Timestamp::as_nanosecond).hash(&mut hasher);
    hasher.write(text);
    hasher.finish()
}

#[cfg(test)]
pub(super) fn line_key(ts: Option<Timestamp>, text: &str) -> u64 {
    key(ts, text.as_bytes())
}
