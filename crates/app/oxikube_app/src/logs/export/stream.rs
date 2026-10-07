//! Reading lines out of a session in bounded chunks, and writing them through the `FsPort`.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedSender;
use oxikube_domain::OxiResult;
use oxikube_ports::{FileChunks, FsPort};

use super::format::ExportFormat;
use crate::logs::{LogEntry, LogReader};

/// A chunk is cut once it holds this many bytes (it may overshoot by one line).
pub const CHUNK_BYTES: usize = 256 * 1024;

/// Most lines one chunk looks at, matching or not, so a filter that rejects almost everything
/// never holds the session's lock for long.
const MAX_SCAN: usize = 16 * 1024;

/// Which lines pass the viewer's filter (E08-S03's search installs its matcher as one). It runs
/// on the thread that writes the file, so it is `Send + Sync`.
pub type LineFilter = Arc<dyn Fn(&LogEntry) -> bool + Send + Sync>;

/// What to read out of a session: the lines in `seqs` that pass `filter`, written in `format`.
/// The seq range is fixed when the spec is made, so a stream that keeps growing the buffer does
/// not make the file longer than what the user was shown; lines that leave the ring meanwhile
/// are skipped.
#[derive(Clone)]
pub struct ExportSpec {
    /// The seqs to export (lines outside the buffer are skipped).
    pub seqs: Range<u64>,
    /// How each line is written.
    pub format: ExportFormat,
    /// Keeps only matching lines; `None` keeps all of `seqs`.
    pub filter: Option<LineFilter>,
}

impl ExportSpec {
    /// The lines `seqs` in `format`, unfiltered.
    pub fn new(seqs: Range<u64>, format: ExportFormat) -> Self {
        Self {
            seqs,
            format,
            filter: None,
        }
    }

    /// Every line the session holds right now.
    pub fn whole_buffer(reader: &LogReader, format: ExportFormat) -> Self {
        let seqs = reader.read(|buffer, _| buffer.first_seq()..buffer.next_seq());
        Self::new(seqs, format)
    }

    /// Keeps only the lines `filter` accepts (`None` removes the filter).
    #[must_use]
    pub fn with_filter(mut self, filter: Option<LineFilter>) -> Self {
        self.filter = filter;
        self
    }

    /// Whether `entry` is part of the export.
    pub fn matches(&self, entry: &LogEntry) -> bool {
        self.filter.as_ref().is_none_or(|filter| filter(entry))
    }

    /// How many lines the spec selects now. Constant time without a filter; with one, every line
    /// is looked at (in locked slices of [`MAX_SCAN`]), so a UI calls it off its thread.
    pub fn count(&self, reader: &LogReader) -> u64 {
        let (first, next) = reader.read(|buffer, _| (buffer.first_seq(), buffer.next_seq()));
        let start = self.seqs.start.max(first);
        let end = self.seqs.end.min(next);
        if start >= end {
            return 0;
        }
        if self.filter.is_none() {
            return end - start;
        }
        let mut cursor = start;
        let mut count = 0;
        while cursor < end {
            let slice_end = (cursor + MAX_SCAN as u64).min(end);
            reader.read(|buffer, _| {
                for entry in buffer.range_seq(cursor..slice_end) {
                    count += u64::from(self.matches(entry));
                }
            });
            cursor = slice_end;
        }
        count
    }
}

/// Lines copied into memory (the clipboard): the text, how many lines it holds, and whether the
/// copy stopped at its size limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopiedText {
    /// The lines, each ending in a newline.
    pub text: String,
    /// How many lines `text` holds.
    pub lines: u64,
    /// Whether more lines matched but would have taken `text` past the limit.
    pub truncated: bool,
}

/// The lines of `spec`, as one string of at most `limit_bytes` (a whole line is always kept or
/// left out): what a copy puts on the clipboard. Reads under one lock, so the limit (a few MB)
/// keeps it within a frame.
pub fn copy_text(reader: &LogReader, spec: &ExportSpec, limit_bytes: usize) -> CopiedText {
    reader.read(|buffer, _| {
        let start = spec.seqs.start.max(buffer.first_seq());
        let end = spec.seqs.end.min(buffer.next_seq());
        let mut copied = CopiedText {
            text: String::new(),
            lines: 0,
            truncated: false,
        };
        if start >= end {
            return copied;
        }
        for entry in buffer.range_seq(start..end) {
            if !spec.matches(entry) {
                continue;
            }
            if copied.text.len() + spec.format.line_len(entry) > limit_bytes {
                copied.truncated = true;
                break;
            }
            spec.format.write_line(entry, &mut copied.text);
            copied.lines += 1;
        }
        copied
    })
}

/// The next chunk of `spec` from `cursor` on, and how many lines are in it; `None` at the end.
fn next_chunk(reader: &LogReader, spec: &ExportSpec, cursor: &mut u64) -> Option<(Vec<u8>, u64)> {
    loop {
        let step = reader.read(|buffer, _| {
            let start = (*cursor).max(spec.seqs.start).max(buffer.first_seq());
            let end = spec.seqs.end.min(buffer.next_seq());
            if start >= end {
                return None;
            }
            let (mut out, mut lines, mut scanned, mut last) = (String::new(), 0, 0, start);
            for entry in buffer.range_seq(start..end) {
                last = entry.seq;
                scanned += 1;
                if spec.matches(entry) {
                    spec.format.write_line(entry, &mut out);
                    lines += 1;
                }
                if out.len() >= CHUNK_BYTES || scanned >= MAX_SCAN {
                    break;
                }
            }
            Some((out.into_bytes(), lines, last + 1))
        });
        let (bytes, lines, next) = step?;
        *cursor = next;
        if !bytes.is_empty() {
            return Some((bytes, lines));
        }
    }
}

/// What a [`chunks`] stream has produced so far; readable from any thread while it runs.
#[derive(Debug, Default)]
pub struct ExportCounters {
    lines: AtomicU64,
    bytes: AtomicU64,
}

impl ExportCounters {
    /// Lines produced so far.
    pub fn lines(&self) -> u64 {
        self.lines.load(Ordering::Relaxed)
    }

    /// Bytes produced so far.
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
}

/// The chunks of `spec` over `reader`: each is at most about [`CHUNK_BYTES`] of formatted lines,
/// read under one short lock, so memory stays flat however long the export. After each chunk the
/// number of lines produced so far is sent to `progress`, and the counters are updated.
pub fn chunks(
    reader: LogReader,
    spec: ExportSpec,
    progress: Option<UnboundedSender<u64>>,
) -> (FileChunks, Arc<ExportCounters>) {
    let counters = Arc::new(ExportCounters::default());
    let state = (reader, spec.seqs.start, spec, counters.clone(), progress);
    let stream = futures::stream::unfold(
        state,
        |(reader, mut cursor, spec, counters, progress)| async move {
            let (bytes, lines) = next_chunk(&reader, &spec, &mut cursor)?;
            counters
                .bytes
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            let total = counters.lines.fetch_add(lines, Ordering::Relaxed) + lines;
            if let Some(progress) = &progress {
                let _ = progress.unbounded_send(total);
            }
            Some((Ok(bytes), (reader, cursor, spec, counters, progress)))
        },
    )
    .boxed();
    (stream, counters)
}

/// What a finished [`save`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportSummary {
    /// Lines written.
    pub lines: u64,
    /// Bytes written.
    pub bytes: u64,
}

/// Writes `spec`'s lines of `reader` to `path` through `fs`, a chunk at a time
/// ([`FsPort::write_stream`]: atomic, nothing appears unless it all succeeds). Await it on a
/// worker, never the UI thread; `progress` gets the running line count after every chunk.
///
/// # Errors
///
/// Whatever the port reports for the path (not writable, a directory, ...).
pub async fn save(
    fs: &dyn FsPort,
    path: &Path,
    reader: LogReader,
    spec: ExportSpec,
    progress: Option<UnboundedSender<u64>>,
) -> OxiResult<ExportSummary> {
    let (stream, counters) = chunks(reader, spec, progress);
    fs.write_stream(path, stream).await?;
    Ok(ExportSummary {
        lines: counters.lines(),
        bytes: counters.bytes(),
    })
}
