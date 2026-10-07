//! What an exported or copied line looks like: its text as written, with the server timestamp and
//! the pod prefix when asked for. Plain Rust.

use super::super::LogEntry;

/// How lines are written. The default is the raw text, one line per line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExportFormat {
    /// Lead each line with its server timestamp ([`timestamp`]), as the viewer draws it.
    pub timestamps: bool,
    /// Lead each line with `pod/container`, which tells the pods of an aggregated view apart.
    pub pod_prefix: bool,
}

impl ExportFormat {
    /// Appends `entry` to `out` as one line, newline included: `[timestamp ][pod/container ]text`.
    pub fn write_line(&self, entry: &LogEntry, out: &mut String) {
        if self.timestamps {
            out.push_str(&timestamp(entry));
            out.push(' ');
        }
        if self.pod_prefix {
            out.push_str(&entry.pod);
            out.push('/');
            out.push_str(&entry.container);
            out.push(' ');
        }
        out.push_str(&entry.text);
        out.push('\n');
    }

    /// The bytes [`write_line`](Self::write_line) adds for `entry` (to size a copy before it is
    /// built).
    pub fn line_len(&self, entry: &LogEntry) -> usize {
        let mut len = entry.text.len() + 1;
        if self.timestamps {
            len += TIMESTAMP_LEN + 1;
        }
        if self.pod_prefix {
            len += entry.pod.len() + entry.container.len() + 2;
        }
        len
    }
}

/// Bytes of [`timestamp`]: `2026-10-07T12:00:00.123Z`.
const TIMESTAMP_LEN: usize = 24;

/// The timestamp column: UTC to the millisecond, fixed width (`2026-10-07T12:00:00.123Z`).
pub fn timestamp(entry: &LogEntry) -> String {
    entry.ts.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// The note for a save or a copy of a buffer that lost lines: `dropped` older lines no longer
/// exist because the buffer keeps only its newest `capacity`. `None` when nothing was dropped.
/// A full-history download from the cluster is a different thing; this says what the file holds.
pub fn truncation_note(dropped: u64, capacity: usize) -> Option<String> {
    (dropped > 0).then(|| {
        format!(
            "The buffer holds the last {capacity} lines; {dropped} older lines were dropped and \
             are not included."
        )
    })
}
