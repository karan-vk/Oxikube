//! Taking lines out of a session: the line format, filters, bounded chunks, and the save through
//! the `FsPort`.

use std::path::Path;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use futures::executor::block_on;
use oxikube_domain::ErrorKind;
use oxikube_domain::log::LogLine;
use oxikube_testkit::{FakeFsPort, Timeline};

use super::{Harness, burst, line, ts};
use crate::logs::export::{
    CHUNK_BYTES, ExportFormat, ExportSpec, LineFilter, chunks, save, timestamp, truncation_note,
};
use crate::logs::{LogConfig, LogEntry, LogReader};

fn entry(i: usize) -> LogEntry {
    LogEntry::new(line(i))
}

fn session(n: usize) -> (Harness, crate::logs::LogSession) {
    let mut h = Harness::new();
    let session = h.follow_flushed(burst(n).keep_open());
    (h, session)
}

fn collect(reader: &LogReader, spec: ExportSpec) -> Vec<Vec<u8>> {
    let (stream, _) = chunks(reader.clone(), spec, None);
    block_on(stream.map(|chunk| chunk.unwrap()).collect())
}

fn text(chunks: &[Vec<u8>]) -> String {
    String::from_utf8(chunks.concat()).unwrap()
}

#[test]
fn a_line_is_its_raw_text_with_optional_timestamp_and_pod_prefix() {
    let e = entry(7);
    let write = |format: ExportFormat| {
        let mut out = String::new();
        format.write_line(&e, &mut out);
        assert_eq!(out.len(), format.line_len(&e), "{format:?} sizes its line");
        out
    };
    assert_eq!(write(ExportFormat::default()), "line 7\n");
    assert_eq!(
        write(ExportFormat {
            timestamps: true,
            pod_prefix: false
        }),
        format!("{} line 7\n", timestamp(&e))
    );
    assert_eq!(
        write(ExportFormat {
            timestamps: false,
            pod_prefix: true
        }),
        "web-0/app line 7\n"
    );
    let both = write(ExportFormat {
        timestamps: true,
        pod_prefix: true,
    });
    assert_eq!(both, "2025-10-09T08:53:20.007Z web-0/app line 7\n");
    assert_eq!(ts(7).to_string(), "2025-10-09T08:53:20.007Z");
}

#[test]
fn the_truncation_note_says_what_the_buffer_kept() {
    assert_eq!(truncation_note(0, 50_000), None);
    let note = truncation_note(1_500, 50_000).unwrap();
    assert!(note.contains("last 50000 lines") && note.contains("1500 older lines"));
}

#[test]
fn the_whole_buffer_exports_in_order_and_counts() {
    let (_h, session) = session(5);
    let spec = ExportSpec::whole_buffer(&session, ExportFormat::default());
    assert_eq!(spec.count(&session), 5);
    assert_eq!(
        text(&collect(&session, spec)),
        "line 0\nline 1\nline 2\nline 3\nline 4\n"
    );
}

#[test]
fn a_seq_range_exports_only_those_lines_and_clamps_to_the_buffer() {
    let (_h, session) = session(10);
    let spec = ExportSpec::new(3..6, ExportFormat::default());
    assert_eq!(spec.count(&session), 3);
    assert_eq!(text(&collect(&session, spec)), "line 3\nline 4\nline 5\n");
    let beyond = ExportSpec::new(8..50, ExportFormat::default());
    assert_eq!(beyond.count(&session), 2);
    assert_eq!(text(&collect(&session, beyond)), "line 8\nline 9\n");
    let none = ExportSpec::new(20..30, ExportFormat::default());
    assert_eq!(none.count(&session), 0);
    assert!(collect(&session, none).is_empty());
}

#[test]
fn the_filter_decides_what_is_written_and_counted() {
    let (_h, session) = session(10);
    let even: LineFilter = Arc::new(|e: &LogEntry| e.seq.is_multiple_of(2));
    let spec = ExportSpec::whole_buffer(&session, ExportFormat::default()).with_filter(Some(even));
    assert_eq!(spec.count(&session), 5);
    assert_eq!(
        text(&collect(&session, spec)),
        "line 0\nline 2\nline 4\nline 6\nline 8\n"
    );
    let nothing: LineFilter = Arc::new(|_| false);
    let spec =
        ExportSpec::whole_buffer(&session, ExportFormat::default()).with_filter(Some(nothing));
    assert_eq!(spec.count(&session), 0);
    assert!(collect(&session, spec).is_empty(), "no empty chunks");
}

#[test]
fn a_long_export_is_cut_into_bounded_chunks_and_reports_progress() {
    let mut h = Harness::with_config(LogConfig {
        buffer_lines: 100_000,
        ..LogConfig::default()
    });
    let wide =
        |i: usize| LogLine::new(ts(i), "web-0", "app", format!("{i:08} {}", "x".repeat(110)));
    let session = h.follow_flushed(Timeline::immediate((0..30_000).map(wide)).keep_open());
    assert_eq!(session.len(), 30_000);
    let spec = ExportSpec::whole_buffer(&session, ExportFormat::default());
    let (tx, mut rx) = unbounded();
    let (stream, counters) = chunks(session.reader(), spec, Some(tx));
    let parts: Vec<Vec<u8>> = block_on(stream.map(|c| c.unwrap()).collect());
    assert!(parts.len() > 10, "{} chunks", parts.len());
    assert!(
        parts.iter().all(|p| p.len() <= CHUNK_BYTES + 200),
        "a chunk stops at its bound (plus the line that crossed it)"
    );
    assert_eq!(counters.lines(), 30_000);
    assert_eq!(
        counters.bytes(),
        parts.iter().map(Vec::len).sum::<usize>() as u64
    );
    let mut reported = Vec::new();
    while let Ok(n) = rx.try_recv() {
        reported.push(n);
    }
    assert_eq!(reported.len(), parts.len(), "one report per chunk");
    assert!(reported.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(reported.last(), Some(&30_000));
}

#[test]
fn lines_that_left_the_ring_meanwhile_are_skipped() {
    let mut h = Harness::with_config(LogConfig {
        buffer_lines: crate::logs::MIN_BUFFER_LINES,
        ..LogConfig::default()
    });
    let n = crate::logs::MIN_BUFFER_LINES + 50;
    let session = h.follow_flushed(burst(n).keep_open());
    // The ring kept the newest MIN lines; a spec made earlier for 0..n only finds those.
    let spec = ExportSpec::new(0..n as u64, ExportFormat::default());
    assert_eq!(spec.count(&session), crate::logs::MIN_BUFFER_LINES as u64);
    let written = text(&collect(&session, spec));
    assert!(written.starts_with("line 50\n"), "{}", &written[..20]);
    assert_eq!(written.lines().count(), crate::logs::MIN_BUFFER_LINES);
}

#[test]
fn save_writes_the_expected_bytes_through_the_fs_port() {
    let (_h, session) = session(3);
    let fs = FakeFsPort::new();
    let spec = ExportSpec::whole_buffer(
        &session,
        ExportFormat {
            timestamps: false,
            pod_prefix: true,
        },
    );
    let summary = block_on(save(
        &fs,
        Path::new("/out/web-0.log"),
        session.reader(),
        spec,
        None,
    ))
    .unwrap();
    let expected = "web-0/app line 0\nweb-0/app line 1\nweb-0/app line 2\n";
    assert_eq!(fs.file("/out/web-0.log").unwrap(), expected.as_bytes());
    assert_eq!((summary.lines, summary.bytes), (3, expected.len() as u64));
}

#[test]
fn an_fs_error_is_returned_and_nothing_is_written() {
    let (_h, session) = session(3);
    let fs = FakeFsPort::new();
    fs.script()
        .write
        .push_err(oxikube_domain::OxiError::forbidden(
            "no permission to write /ro/x.log",
        ));
    let spec = ExportSpec::whole_buffer(&session, ExportFormat::default());
    let error = block_on(save(
        &fs,
        Path::new("/ro/x.log"),
        session.reader(),
        spec,
        None,
    ))
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert!(fs.file("/ro/x.log").is_none());
}

#[test]
fn a_copy_is_capped_at_whole_lines_and_says_so() {
    let (_h, session) = session(100);
    let spec = ExportSpec::new(10..20, ExportFormat::default());
    let all = crate::logs::export::copy_text(&session, &spec, usize::MAX);
    assert_eq!((all.lines, all.truncated, all.left_out), (10, false, 0));
    assert!(all.text.starts_with("line 10\n") && all.text.ends_with("line 19\n"));
    // "line 10\n" is 8 bytes: a limit of 20 holds two lines, never a cut one.
    let capped = crate::logs::export::copy_text(&session, &spec, 20);
    assert_eq!((capped.lines, capped.truncated), (2, true));
    assert_eq!(capped.left_out, 8, "the rest of the selection is counted");
    assert_eq!(capped.text, "line 10\nline 11\n");
    let odd: LineFilter = Arc::new(|e: &LogEntry| e.seq % 2 == 1);
    let filtered = crate::logs::export::copy_text(&session, &spec.with_filter(Some(odd)), 1_000);
    assert_eq!(
        filtered.text,
        "line 11\nline 13\nline 15\nline 17\nline 19\n"
    );
    let nothing = crate::logs::export::copy_text(
        &session,
        &ExportSpec::new(500..600, ExportFormat::default()),
        1_000,
    );
    assert_eq!((nothing.lines, nothing.text.as_str()), (0, ""));
}
