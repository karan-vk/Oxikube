//! Writing the audit log to a file (`audit::Export`): JSONL or CSV, streamed.
//!
//! [`export`] pages through the log with the viewer's filters and hands
//! [`FsPort::write_stream`](oxikube_ports::FsPort::write_stream) one formatted page at a time, so a
//! million records never sit in memory and nothing appears at the destination unless the whole
//! export succeeded. Records carry no bodies (`oxikube_domain::audit`) and were redacted when they
//! were written, so the export adds nothing a record did not already say.
//!
//! # CSV
//!
//! Cells are quoted when they hold a comma, a quote or a line break (quotes doubled). A cell that
//! starts with `=`, `+`, `-`, `@`, a tab or a carriage return would be run as a formula by a
//! spreadsheet, so it gets a leading `'` ([`neutralise`]); the cell is still readable and no
//! formula runs.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt as _;
use jiff::Timestamp;
use oxikube_domain::audit::{AuditDenial, AuditExportFormat, AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{AuditCursor, FileChunks, FsPort};

use super::query::{AuditFilters, AuditReader};

/// Records formatted per chunk: one query, one write.
pub const EXPORT_PAGE: usize = 500;

/// The CSV header row.
pub const CSV_HEADER: &str = "time,cluster,namespace,verb,kind,name,outcome,initiator,dry_run,who,command,batch_id,reason,detail";

/// What a finished [`export`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportSummary {
    /// Records written.
    pub records: u64,
    /// Bytes written.
    pub bytes: u64,
}

/// `cell` made safe to open in a spreadsheet: a leading `=`, `+`, `-`, `@`, tab or carriage return
/// gets a `'` in front. Borrowed when nothing was needed.
pub fn neutralise(cell: &str) -> Cow<'_, str> {
    if cell.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        Cow::Owned(format!("'{cell}"))
    } else {
        Cow::Borrowed(cell)
    }
}

fn push_cell(out: &mut String, cell: &str) {
    let cell = neutralise(cell);
    if cell.contains([',', '"', '\n', '\r']) {
        out.push('"');
        out.push_str(&cell.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(&cell);
    }
}

/// The word for an outcome.
pub fn outcome_text(outcome: AuditOutcome) -> &'static str {
    match outcome {
        AuditOutcome::Succeeded => "succeeded",
        AuditOutcome::Failed => "failed",
        AuditOutcome::Denied => "denied",
        AuditOutcome::Cancelled => "cancelled",
    }
}

/// The word for an initiator.
pub const fn initiator_text(initiator: Initiator) -> &'static str {
    match initiator {
        Initiator::Ui => "ui",
        Initiator::Command => "command",
        Initiator::Agent => "agent",
        Initiator::Plugin => "plugin",
    }
}

/// The word for a denial reason.
pub fn denial_text(denial: AuditDenial) -> &'static str {
    match denial {
        AuditDenial::ReadOnly => "read_only",
        AuditDenial::UserCancelled => "user_cancelled",
        AuditDenial::Policy => "policy",
    }
}

/// Appends `record` as one CSV row (with its line break) to `out`.
pub fn write_csv_row(record: &AuditRecord, out: &mut String) {
    let cells: [Cow<'_, str>; 14] = [
        Cow::Owned(record.ts.to_string()),
        Cow::Owned(record.cluster.to_string()),
        Cow::Borrowed(record.target.namespace.as_deref().unwrap_or("")),
        Cow::Borrowed(record.verb.map_or("", |verb| verb.as_str())),
        Cow::Borrowed(&record.target.gvk.kind),
        Cow::Borrowed(&record.target.name),
        Cow::Borrowed(outcome_text(record.outcome)),
        Cow::Borrowed(initiator_text(record.initiator)),
        Cow::Borrowed(if record.dry_run { "true" } else { "false" }),
        Cow::Borrowed(&record.who),
        Cow::Borrowed(&record.cmd),
        Cow::Borrowed(record.batch_id.as_deref().unwrap_or("")),
        Cow::Borrowed(record.denial.map_or("", denial_text)),
        Cow::Borrowed(record.detail.as_deref().unwrap_or("")),
    ];
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_cell(out, cell);
    }
    out.push('\n');
}

/// Appends `record` as one JSON line (with its line break) to `out`: the record's own serde shape.
pub fn write_jsonl_line(record: &AuditRecord, out: &mut String) {
    // A record is plain data with string keys; serialising it cannot fail.
    if let Ok(line) = serde_json::to_string(record) {
        out.push_str(&line);
        out.push('\n');
    }
}

/// Formats `records` in `format`, with the CSV header first when `header`.
pub fn format_page(format: AuditExportFormat, records: &[AuditRecord], header: bool) -> String {
    let mut out = String::with_capacity(records.len() * 192);
    if header && format == AuditExportFormat::Csv {
        out.push_str(CSV_HEADER);
        out.push('\n');
    }
    for record in records {
        match format {
            AuditExportFormat::Jsonl => write_jsonl_line(record, &mut out),
            AuditExportFormat::Csv => write_csv_row(record, &mut out),
        }
    }
    out
}

/// The file name the save panel suggests: `oxikube-audit-<time>.<ext>`.
pub fn suggested_file_name(format: AuditExportFormat, now: Timestamp) -> String {
    format!(
        "oxikube-audit-{}.{}",
        now.strftime("%Y%m%d-%H%M%S"),
        format.extension()
    )
}

/// What a running export has produced; readable from any thread.
#[derive(Debug, Default)]
pub struct ExportCounters {
    records: AtomicU64,
    bytes: AtomicU64,
}

struct Paging {
    reader: AuditReader,
    filters: AuditFilters,
    now: Timestamp,
    format: AuditExportFormat,
    cursor: Option<AuditCursor>,
    first: bool,
    done: bool,
    counters: Arc<ExportCounters>,
}

/// The chunks of an export of what `filters` match, as of `now`: one per page, read on demand.
pub fn chunks(
    reader: AuditReader,
    filters: AuditFilters,
    now: Timestamp,
    format: AuditExportFormat,
) -> (FileChunks, Arc<ExportCounters>) {
    let counters = Arc::new(ExportCounters::default());
    let state = Paging {
        reader,
        filters,
        now,
        format,
        cursor: None,
        first: true,
        done: false,
        counters: counters.clone(),
    };
    let stream = futures::stream::unfold(state, |mut state| async move {
        if state.done {
            return None;
        }
        let page = match state
            .reader
            .page(&state.filters, state.now, state.cursor, EXPORT_PAGE)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                state.done = true;
                return Some((Err(error), state));
            }
        };
        state.cursor = page.next;
        state.done = page.next.is_none();
        let header = std::mem::take(&mut state.first);
        if page.records.is_empty() && !header {
            return None;
        }
        let text = format_page(state.format, &page.records, header);
        state
            .counters
            .records
            .fetch_add(page.records.len() as u64, Ordering::Relaxed);
        state
            .counters
            .bytes
            .fetch_add(text.len() as u64, Ordering::Relaxed);
        Some((Ok(text.into_bytes()), state))
    })
    .boxed();
    (stream, counters)
}

/// Writes what `filters` match to `path` through `fs`, a page at a time. Await it on a worker,
/// never the UI thread.
///
/// # Errors
///
/// The store's error while reading, or the port's while writing (not writable, a directory);
/// nothing is left at `path` then.
pub async fn export(
    fs: &dyn FsPort,
    path: &Path,
    reader: AuditReader,
    filters: AuditFilters,
    now: Timestamp,
    format: AuditExportFormat,
) -> OxiResult<ExportSummary> {
    let (stream, counters) = chunks(reader, filters, now, format);
    fs.write_stream(path, stream)
        .await
        .map_err(|error| OxiError::internal(format!("writing the audit export: {error}")))?;
    Ok(ExportSummary {
        records: counters.records.load(Ordering::Relaxed),
        bytes: counters.bytes.load(Ordering::Relaxed),
    })
}

#[cfg(test)]
mod tests;
