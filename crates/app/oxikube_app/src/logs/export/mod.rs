//! Taking lines out of a [`LogSession`](super::LogSession): the format of a saved or copied line,
//! and the chunked read that feeds [`FsPort::write_stream`](oxikube_ports::FsPort::write_stream)
//! (E08-S06).
//!
//! | Piece | Where |
//! |---|---|
//! | `[timestamp ][pod/container ]text` per line, the truncation note | [`ExportFormat`], [`timestamp`], [`truncation_note`] (`format`) |
//! | which lines: a seq range plus the viewer's filter | [`ExportSpec`], [`LineFilter`] (`stream`) |
//! | a copy into memory, capped | [`copy_text`], [`CopiedText`] (`stream`) |
//! | the bounded chunks, their counters and the write | [`chunks`], [`ExportCounters`], [`save`], [`ExportSummary`] (`stream`) |
//!
//! Memory stays flat: a chunk is about [`CHUNK_BYTES`] of formatted lines read under one short
//! lock, and the port writes it before the next is made, so exporting a 5 000 000 line buffer
//! costs a quarter of a megabyte at a time. The caller runs [`save`] on a worker (the viewer
//! uses the Tokio bridge), never the UI thread. A save is a local file the user chose, not a
//! cluster mutation: it does not go through `MutationGuard`.

mod format;
mod stream;

pub use format::{ExportFormat, timestamp, truncation_note, utc_millis};
pub use stream::{
    CHUNK_BYTES, CopiedText, ExportCounters, ExportSpec, ExportSummary, LineFilter, chunks,
    copy_text, save,
};
