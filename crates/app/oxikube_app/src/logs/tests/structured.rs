//! A session stores the level of each structured line as it commits it, and nothing for plain text.

use oxikube_domain::log::{LogLevel, LogLine};
use oxikube_testkit::Timeline;

use super::{Harness, ts};
use crate::logs::LevelFilter;

fn text_line(i: usize, text: &str) -> LogLine {
    LogLine::new(ts(i), "web-0", "app", text)
}

#[test]
fn committed_entries_carry_the_level_of_their_json_and_none_for_text() {
    let mut h = Harness::new();
    let lines = [
        "=== booting ===",
        r#"{"level":"info","ts":1760000000.1,"msg":"ready"}"#,
        r#"{"level":50,"time":1760000000100,"msg":"pino error"}"#,
        r#"{"msg":"no level here"}"#,
        r#"{"level":"warn","msg":"cut o"#,
        "[1,2]",
    ];
    let timeline = Timeline::immediate(lines.iter().enumerate().map(|(i, t)| text_line(i, t)));
    let session = h.follow_flushed(timeline.keep_open());
    let got: Vec<_> = session.read(|buffer, _| buffer.iter().map(|e| e.level).collect());
    assert_eq!(
        got,
        [
            None,
            Some(LogLevel::Info),
            Some(LogLevel::Error),
            Some(LogLevel::Unknown),
            None,
            None,
        ]
    );
}

#[test]
fn a_line_cut_by_the_size_cap_is_text_even_if_it_started_as_json() {
    let mut h = Harness::new();
    let long = format!(
        r#"{{"level":"error","msg":"{}"}}"#,
        "x".repeat(oxikube_domain::log::MAX_LOG_LINE_BYTES)
    );
    let session = h.follow_flushed(Timeline::immediate([text_line(0, &long)]).keep_open());
    session.read(|buffer, _| {
        let entry = buffer.get(0).unwrap();
        assert!(entry.truncated);
        assert_eq!(entry.level, None);
    });
}

#[test]
fn a_level_filter_admits_entries_by_their_chip() {
    use oxikube_domain::log::LevelChip;
    let mut h = Harness::new();
    let lines = [
        "plain",
        r#"{"level":"debug","msg":"d"}"#,
        r#"{"level":"error","msg":"e"}"#,
    ];
    let timeline = Timeline::immediate(lines.iter().enumerate().map(|(i, t)| text_line(i, t)));
    let session = h.follow_flushed(timeline.keep_open());
    let mut filter = LevelFilter::all();
    filter.set(LevelChip::Debug, false);
    filter.set(LevelChip::Text, false);
    let shown: Vec<u64> = session.read(|buffer, _| {
        buffer
            .iter()
            .filter(|e| filter.admits(e))
            .map(|e| e.seq)
            .collect()
    });
    assert_eq!(shown, [2]);
}
