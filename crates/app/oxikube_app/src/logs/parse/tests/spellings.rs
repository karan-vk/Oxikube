//! The spellings of levels and times.

use oxikube_domain::log::LogLevel::{self, Debug, Error, Fatal, Info, Trace, Unknown, Warn};
use serde_json::json;

use super::super::{normalise_level, parse_time};
use super::ts;

#[test]
fn level_names() {
    for (name, want) in [
        ("trace", Trace),
        ("DEBUG", Debug),
        ("Info", Info),
        ("information", Info),
        ("inf", Info),
        ("INF", Info),
        ("infinity", Unknown),
        ("NaN", Unknown),
        ("NOTICE", Info),
        ("warn", Warn),
        ("WARNING", Warn),
        ("err", Error),
        ("ERROR", Error),
        ("fatal", Fatal),
        ("panic", Fatal),
        ("dpanic", Fatal),
        ("critical", Fatal),
        ("EMERGENCY", Fatal),
        (" info ", Info),
        ("", Unknown),
        ("loud", Unknown),
        ("this is not a level but a sentence", Unknown),
    ] {
        assert_eq!(normalise_level(&json!(name)), want, "{name:?}");
    }
}

#[test]
fn numeric_levels_are_bunyan_and_pino_decades() {
    for (n, want) in [
        (10, Trace),
        (20, Debug),
        (30, Info),
        (35, Info),
        (40, Warn),
        (50, Error),
        (60, Fatal),
        (70, Fatal),
        (5, Unknown),
        (0, Unknown),
    ] {
        assert_eq!(normalise_level(&json!(n)), want, "{n}");
    }
    assert_eq!(normalise_level(&json!(30.0)), Info);
    assert_eq!(normalise_level(&json!("40")), Warn, "a numeric string");
    for other in [json!(null), json!(true), json!({"a":1}), json!([30])] {
        assert_eq!(normalise_level(&other), Unknown, "{other}");
    }
    assert!(LogLevel::KNOWN.iter().all(|l| *l != Unknown));
}

#[test]
fn rfc3339_times() {
    let want = Some(ts("2023-10-07T09:20:00.123Z"));
    for text in [
        "2023-10-07T09:20:00.123Z",
        "2023-10-07T18:20:00.123+09:00",
        "2023-10-07T18:20:00.123+0900",
        "2023-10-07t09:20:00.123z",
        "2023-10-07T09:20:00.123",
        "2023-10-07 09:20:00.123",
        " 2023-10-07T09:20:00.123Z ",
    ] {
        assert_eq!(parse_time(&json!(text)), want, "{text:?}");
    }
}

#[test]
fn epoch_seconds_millis_micros_nanos_and_floats() {
    let want = Some(ts("2023-10-07T09:20:00.123Z"));
    assert_eq!(parse_time(&json!(1_696_670_400_123_i64)), want, "millis");
    assert_eq!(
        parse_time(&json!(1_696_670_400_123_000_i64)),
        want,
        "micros"
    );
    assert_eq!(
        parse_time(&json!(1_696_670_400_123_000_000_i64)),
        want,
        "nanos"
    );
    assert_eq!(
        parse_time(&json!(1_696_670_400)),
        Some(ts("2023-10-07T09:20:00Z")),
        "seconds"
    );
    let float = parse_time(&json!(1_696_670_400.123)).unwrap();
    assert!(
        float
            .as_nanosecond()
            .abs_diff(ts("2023-10-07T09:20:00.123Z").as_nanosecond())
            < 1_000,
        "float seconds"
    );
    assert_eq!(parse_time(&json!("1696670400123")), want, "numeric string");
    assert_eq!(
        parse_time(&json!("1696670400.5")),
        Some(ts("2023-10-07T09:20:00.5Z"))
    );
}

#[test]
fn what_is_not_a_time_is_none_and_the_text_is_kept() {
    for value in [
        json!("yesterday"),
        json!(""),
        json!(0.5),
        json!(42),
        json!(-1),
        json!(null),
        json!(true),
        json!(1e30),
    ] {
        assert_eq!(parse_time(&value), None, "{value}");
    }
    let record = super::super::parse_line(r#"{"msg":"m","time":"4ms ago"}"#).unwrap();
    let time = record.time.unwrap();
    assert_eq!(time.timestamp, None);
    assert_eq!(
        time.display(),
        "4ms ago",
        "the original text is what is shown"
    );
}

#[test]
fn display_rounds_to_the_millisecond() {
    let record = super::super::parse_line(r#"{"time":1696670400.1239996}"#).unwrap();
    assert_eq!(record.time.unwrap().display(), "09:20:00.124");
    let record = super::super::parse_line(r#"{"time":"2023-10-07T09:20:00.123456Z"}"#).unwrap();
    let time = record.time.unwrap();
    assert_eq!(time.display(), "09:20:00.123");
}
