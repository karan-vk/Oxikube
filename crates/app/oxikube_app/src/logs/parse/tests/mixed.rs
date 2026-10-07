//! Mixed streams: what is and is not a record, and the bounds.

use oxikube_domain::log::LogLevel;

use super::super::{MAX_PARSE_BYTES, classify, parse_line, pretty};

#[test]
fn only_complete_json_objects_are_records() {
    for text in [
        "starting up...",
        "",
        "   ",
        "[1,2,3]",
        "42",
        "\"just a string\"",
        "null",
        "true",
        r#"[{"level":"info"}]"#,
        // Cut mid-object (a line cut by `limit_bytes`), and text after the object.
        r#"{"level":"info","msg":"cut o"#,
        r#"{"level":"info","msg":"x"} trailing"#,
        r#"2026-10-07T12:00:00Z {"level":"info"}"#,
        "{not json}",
    ] {
        assert!(parse_line(text).is_none(), "{text:?} is plain text");
        assert_eq!(classify(text, false), None, "{text:?}");
    }
}

#[test]
fn detection_is_per_line_and_tolerates_leading_blanks() {
    let stream = [
        "=== booting service ===",
        r#"{"level":"info","msg":"ready"}"#,
        "",
        r#"  {"level":"error","msg":"bad"}"#,
        "java.lang.NullPointerException",
        "{}",
    ];
    let seen: Vec<_> = stream.iter().map(|line| classify(line, false)).collect();
    assert_eq!(
        seen,
        [
            None,
            Some(LogLevel::Info),
            None,
            Some(LogLevel::Error),
            None,
            Some(LogLevel::Unknown),
        ]
    );
}

#[test]
fn nested_objects_and_escaped_newlines_are_fine() {
    let record =
        parse_line(r#"{"level":"warn","msg":"line one\nline two","ctx":{"a":{"b":[1,2]}}}"#)
            .unwrap();
    assert_eq!(record.message.as_deref(), Some("line one\nline two"));
    assert_eq!(record.message_line(), "line one line two");
    assert_eq!(record.summary(100), r#"ctx={"a":{"b":[1,2]}}"#);
}

#[test]
fn a_truncated_line_is_text_even_when_it_looks_like_json() {
    let complete = r#"{"level":"info","msg":"x"}"#;
    assert_eq!(classify(complete, false), Some(LogLevel::Info));
    assert_eq!(classify(complete, true), None);
}

#[test]
fn a_line_beyond_the_size_cap_is_raw_text() {
    let big = format!(
        r#"{{"level":"info","msg":"{}"}}"#,
        "x".repeat(MAX_PARSE_BYTES)
    );
    assert!(big.len() > MAX_PARSE_BYTES);
    assert!(parse_line(&big).is_none());
    let ok = format!(
        r#"{{"level":"info","msg":"{}"}}"#,
        "x".repeat(MAX_PARSE_BYTES - 64)
    );
    assert!(parse_line(&ok).is_some());
}

#[test]
fn huge_values_are_cut_in_the_summary_not_in_the_record() {
    let value = "v".repeat(5_000);
    let record = parse_line(&format!(r#"{{"msg":"m","blob":"{value}"}}"#)).unwrap();
    assert_eq!(record.fields[0].1.as_str().unwrap().len(), 5_000);
    assert!(record.summary(500).chars().count() <= 500);
}

#[test]
fn pretty_prints_objects_in_source_order() {
    let text = r#"{"msg":"hi","level":"info","nested":{"k":[1,2]}}"#;
    assert_eq!(
        pretty(text).unwrap(),
        "{\n  \"msg\": \"hi\",\n  \"level\": \"info\",\n  \"nested\": {\n    \"k\": [\n      1,\n      2\n    ]\n  }\n}"
    );
    assert_eq!(pretty("plain"), None);
    assert_eq!(pretty("[1]"), None);
    assert_eq!(pretty(r#"{"cut":"#), None);
}

#[test]
fn a_custom_field_map_reads_other_names() {
    use super::super::{FieldMap, parse_line_with};
    let map = FieldMap::default().with_first(&["sev"], &["when"], &["what"]);
    let record = parse_line_with(
        &map,
        r#"{"sev":"error","when":"2023-10-07T09:20:00Z","what":"x","level":"info"}"#,
    )
    .unwrap();
    assert_eq!(record.level, LogLevel::Error);
    assert_eq!(record.message.as_deref(), Some("x"));
    assert!(record.time.unwrap().timestamp.is_some());
    // The default names are still tried after, and an unused one stays a field.
    assert_eq!(record.fields.len(), 1);
}

#[test]
fn field_names_match_ignoring_case_and_alias_order_is_by_priority() {
    let record =
        parse_line(r#"{"Severity":"WARNING","Message":"m","Timestamp":"2023-10-07T09:20:00Z"}"#)
            .unwrap();
    assert_eq!(record.level, LogLevel::Warn);
    assert_eq!(record.message.as_deref(), Some("m"));
    assert!(record.fields.is_empty());
    // `level` outranks `severity`; the loser stays a field.
    let record = parse_line(r#"{"severity":"error","level":"info","msg":"m"}"#).unwrap();
    assert_eq!(record.level, LogLevel::Info);
    assert_eq!(record.fields[0].0, "severity");
}

#[test]
fn a_line_without_the_usual_fields_is_still_a_record() {
    let record = parse_line(r#"{"user":"kim","action":"login"}"#).unwrap();
    assert_eq!(record.level, LogLevel::Unknown);
    assert!(record.time.is_none());
    assert!(record.message.is_none());
    assert_eq!(record.summary(100), "user=kim action=login");
    // A non-text message stays a field, so nothing is lost.
    let record = parse_line(r#"{"msg":{"a":1}}"#).unwrap();
    assert!(record.message.is_none());
    assert_eq!(record.fields.len(), 1);
}
