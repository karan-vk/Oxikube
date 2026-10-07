//! Real sample lines from zap, logrus, bunyan and pino (committed under `fixtures/`): level,
//! time and message come out, and the rest stays a field.

use oxikube_domain::log::LogLevel::{Debug, Error, Fatal, Info, Trace, Warn};
use serde_json::json;

use super::{levels, records, ts};

fn field<'a>(record: &'a super::LogRecord, name: &str) -> Option<&'a serde_json::Value> {
    record
        .fields
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
}

fn keys(record: &super::LogRecord) -> Vec<&str> {
    record.fields.iter().map(|(k, _)| k.as_str()).collect()
}

#[test]
fn zap() {
    let records = records(include_str!("fixtures/zap.log"));
    assert_eq!(levels(&records), [Info, Debug, Warn, Error, Fatal, Info]);
    let first = &records[0];
    assert_eq!(first.message.as_deref(), Some("starting server"));
    // `ts` is epoch seconds as a float.
    let time = first.time.as_ref().unwrap();
    let got = time.timestamp.unwrap();
    let want = ts("2023-10-07T09:20:00.123456Z");
    assert!(
        got.as_nanosecond().abs_diff(want.as_nanosecond()) < 1_000,
        "{got} vs {want}"
    );
    assert_eq!(time.display(), "09:20:00.123");
    assert_eq!(keys(first), ["caller", "addr", "version"]);
    assert_eq!(field(first, "addr"), Some(&json!(":8080")));
    // An integer `ts`, and the ISO 8601 encoder with a `+0900` offset.
    assert_eq!(
        records[4].time.as_ref().unwrap().timestamp,
        Some(ts("2023-10-07T09:20:03Z"))
    );
    assert_eq!(
        records[5].time.as_ref().unwrap().timestamp,
        Some(ts("2023-10-07T09:00:04.456Z"))
    );
    // Stack traces stay in the fields, newlines intact.
    assert!(
        field(&records[3], "stacktrace")
            .unwrap()
            .as_str()
            .unwrap()
            .contains('\n')
    );
}

#[test]
fn logrus() {
    let records = records(include_str!("fixtures/logrus.log"));
    assert_eq!(
        levels(&records),
        [Info, Warn, Error, Trace, Debug, Fatal, Fatal]
    );
    assert_eq!(records[0].message.as_deref(), Some("Listening on :8080"));
    assert_eq!(
        records[0].time.as_ref().unwrap().timestamp,
        Some(ts("2023-10-07T09:20:00Z"))
    );
    assert!(records[0].fields.is_empty());
    // A `+09:00` offset with nanoseconds.
    assert_eq!(
        records[1].time.as_ref().unwrap().timestamp,
        Some(ts("2023-10-07T00:20:01.123456789Z"))
    );
    assert_eq!(keys(&records[1]), ["animal"]);
    assert_eq!(keys(&records[2]), ["err", "retries"]);
}

#[test]
fn bunyan() {
    let records = records(include_str!("fixtures/bunyan.log"));
    assert_eq!(levels(&records), [Info, Trace, Debug, Warn, Error, Fatal]);
    let first = &records[0];
    assert_eq!(first.message.as_deref(), Some("server listening"));
    assert_eq!(
        first.time.as_ref().unwrap().timestamp,
        Some(ts("2023-10-07T09:20:00.123Z"))
    );
    // name, hostname, pid and v are not special: they are fields.
    assert_eq!(keys(first), ["name", "hostname", "pid", "v"]);
    assert_eq!(field(first, "pid"), Some(&json!(18)));
    assert_eq!(field(&records[4], "err").unwrap()["message"], "boom");
}

#[test]
fn pino() {
    let records = records(include_str!("fixtures/pino.log"));
    assert_eq!(
        levels(&records),
        [Info, Debug, Info, Warn, Error, Fatal, Trace]
    );
    let first = &records[0];
    assert_eq!(first.message.as_deref(), Some("server started"));
    // `time` is epoch milliseconds.
    assert_eq!(
        first.time.as_ref().unwrap().timestamp,
        Some(ts("2023-10-07T09:20:00.123Z"))
    );
    assert_eq!(keys(first), ["pid", "hostname", "port"]);
    // The message may come after the fields.
    assert_eq!(records[2].message.as_deref(), Some("request completed"));
    assert_eq!(field(&records[2], "req").unwrap()["url"], "/health");
}

#[test]
fn the_collapsed_summary_is_key_value_pairs() {
    let records = records(include_str!("fixtures/pino.log"));
    assert_eq!(records[0].summary(200), "pid=1 hostname=web-0 port=3000");
    assert_eq!(
        records[2].summary(400),
        r#"pid=1 hostname=web-0 req={"id":7,"method":"GET","url":"/health"} res={"statusCode":200} responseTime=3"#
    );
    // Cut to the width asked for, ending in an ellipsis.
    let short = records[2].summary(20);
    assert_eq!(short.chars().count(), 20);
    assert!(short.ends_with('…'));
    // A value with blanks is quoted.
    let rec = super::parse_line(r#"{"msg":"m","reason":"two words","empty":""}"#).unwrap();
    assert_eq!(rec.summary(100), r#"reason="two words" empty="""#);
}
