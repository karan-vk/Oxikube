//! The line reader: timestamp prefix, bounds, partial reads.

use futures::TryStreamExt;
use futures::stream;
use jiff::Timestamp;
use oxikube_domain::log::MAX_LOG_LINE_BYTES;

use crate::logs::line::{Line, LineReader, MAX_RAW_LINE_BYTES, line_key};
use crate::logs::source::Reader;

fn reader(chunks: Vec<&'static [u8]>) -> LineReader {
    let inner: Reader = Box::pin(
        stream::iter(
            chunks
                .into_iter()
                .map(|c| Ok::<_, std::io::Error>(c.to_vec())),
        )
        .into_async_read(),
    );
    LineReader::new(inner)
}

async fn all(mut reader: LineReader) -> Vec<Line> {
    let mut out = Vec::new();
    while let Some(line) = reader.next_line().await.expect("read") {
        out.push(line);
    }
    out
}

fn stamp(s: &str) -> Timestamp {
    s.parse().expect("timestamp")
}

#[tokio::test]
async fn splits_the_timestamp_from_the_text() {
    let lines = all(reader(vec![
        b"2026-10-03T12:00:00.123456789Z hello world\n2026-10-03T12:00:01Z second\n",
    ]))
    .await;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].ts, Some(stamp("2026-10-03T12:00:00.123456789Z")));
    assert_eq!(lines[0].text, "hello world");
    assert_eq!(lines[1].ts, Some(stamp("2026-10-03T12:00:01Z")));
    assert_eq!(lines[1].text, "second");
}

#[tokio::test]
async fn a_line_split_across_reads_is_joined() {
    let lines = all(reader(vec![
        b"2026-10-03T12:00:00Z par",
        b"tial li",
        b"ne\n2026-10-03T12:00:01Z next\n",
    ]))
    .await;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].text, "partial line");
    assert_eq!(lines[1].text, "next");
}

#[tokio::test]
async fn a_last_line_without_a_newline_is_kept() {
    let lines = all(reader(vec![b"2026-10-03T12:00:00Z tail"])).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "tail");
}

#[tokio::test]
async fn blank_lines_and_carriage_returns() {
    let lines = all(reader(vec![
        b"2026-10-03T12:00:00Z \n2026-10-03T12:00:01Z windows\r\n2026-10-03T12:00:02Z\n",
    ]))
    .await;
    let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["", "windows", ""]);
    assert!(lines.iter().all(|l| l.ts.is_some()));
}

#[tokio::test]
async fn a_line_without_a_prefix_has_no_timestamp() {
    let lines = all(reader(vec![b"no prefix here\n12345 not a stamp\n"])).await;
    assert_eq!(lines[0].ts, None);
    assert_eq!(lines[0].text, "no prefix here");
    assert_eq!(lines[1].ts, None);
    assert_eq!(lines[1].text, "12345 not a stamp");
}

#[tokio::test]
async fn invalid_utf8_is_replaced_not_fatal() {
    let lines = all(reader(vec![b"2026-10-03T12:00:00Z bad \xff byte\n"])).await;
    assert_eq!(lines[0].text, "bad \u{fffd} byte");
}

#[tokio::test]
async fn an_overlong_line_is_bounded_and_flagged() {
    // 4 MB without a newline, then a normal line: memory stays bounded and reading resumes.
    let big = vec![b'a'; 4 * 1024 * 1024];
    let big: &'static [u8] = Box::leak(big.into_boxed_slice());
    let mut chunks = vec![b"2026-10-03T12:00:00Z ".as_slice(), big];
    chunks.push(b"\n2026-10-03T12:00:01Z after\n");
    let lines = all(reader(chunks)).await;
    assert_eq!(lines.len(), 2);
    assert!(lines[0].cut);
    assert!(lines[0].text.len() <= MAX_RAW_LINE_BYTES);
    assert!(lines[0].text.len() >= MAX_LOG_LINE_BYTES);
    assert!(!lines[1].cut);
    assert_eq!(lines[1].text, "after");
}

#[tokio::test]
async fn the_key_depends_on_timestamp_and_text() {
    let lines = all(reader(vec![
        b"2026-10-03T12:00:00Z retry\n2026-10-03T12:00:01Z retry\n2026-10-03T12:00:00Z retry\n",
    ]))
    .await;
    assert_ne!(lines[0].key, lines[1].key, "same text, different time");
    assert_eq!(lines[0].key, lines[2].key);
    assert_eq!(
        lines[0].key,
        line_key(Some(stamp("2026-10-03T12:00:00Z")), "retry")
    );
}
