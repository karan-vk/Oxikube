//! The Prometheus HTTP API's JSON envelope, decoded straight into domain-facing types.
//!
//! Responses are parsed from bytes into typed structs (no intermediate `serde_json::Value`
//! tree), so a large matrix is decoded once. `NaN`, `+Inf` and `-Inf` sample values arrive as
//! strings and become the matching `f64`s.

use std::collections::BTreeMap;

use jiff::Timestamp;
use oxikube_ports::{PromqlError, PromqlSeries, PromqlValue};
use serde::Deserialize;

/// `{"status":"success","data":{...}}` or `{"status":"error","errorType":..,"error":..}`.
#[derive(Deserialize)]
struct Envelope {
    status: String,
    data: Option<Data>,
    #[serde(rename = "errorType")]
    error_type: Option<String>,
    error: Option<String>,
}

/// `data`, tagged by `resultType`. Unknown result types fail to decode and are reported as a bad
/// response.
#[derive(Deserialize)]
#[serde(tag = "resultType", content = "result", rename_all = "lowercase")]
enum Data {
    Vector(Vec<InstantItem>),
    Matrix(Vec<RangeItem>),
    Scalar(RawSample),
    String(serde::de::IgnoredAny),
}

/// A sample on the wire: `[unix_seconds, "value"]`.
#[derive(Deserialize)]
struct RawSample(f64, String);

#[derive(Deserialize)]
struct InstantItem {
    metric: BTreeMap<String, String>,
    value: RawSample,
}

#[derive(Deserialize)]
struct RangeItem {
    metric: BTreeMap<String, String>,
    values: Vec<RawSample>,
}

/// What the server said, once the body is read.
pub(crate) enum Reply {
    /// A successful query.
    Success(Vec<PromqlSeries>),
    /// `status: "error"` with its `errorType` and `error` text.
    Failure {
        /// Prometheus's `errorType` (`bad_data`, `execution`, `timeout`, ...), possibly numeric
        /// text on VictoriaMetrics.
        error_type: String,
        /// The `error` text.
        message: String,
    },
}

/// Decodes a response body. `Err` means the body is not a Prometheus API response at all.
pub(crate) fn parse(body: &[u8]) -> Result<Reply, PromqlError> {
    let envelope: Envelope = serde_json::from_slice(body)
        .map_err(|e| PromqlError::BadResponse(format!("not a Prometheus API response ({e})")))?;
    match envelope.status.as_str() {
        "success" => {
            let data = envelope.data.ok_or_else(|| {
                PromqlError::BadResponse("success response without `data`".into())
            })?;
            convert(data).map(Reply::Success)
        }
        "error" => Ok(Reply::Failure {
            error_type: envelope.error_type.unwrap_or_default(),
            message: envelope.error.unwrap_or_default(),
        }),
        other => Err(PromqlError::BadResponse(format!(
            "unknown response status `{other}`"
        ))),
    }
}

fn convert(data: Data) -> Result<Vec<PromqlSeries>, PromqlError> {
    match data {
        Data::Vector(items) => items
            .into_iter()
            .map(|item| series(item.metric, std::iter::once(item.value)))
            .collect(),
        Data::Matrix(items) => items
            .into_iter()
            .map(|item| series(item.metric, item.values))
            .collect(),
        Data::Scalar(sample) => series(BTreeMap::new(), std::iter::once(sample)).map(|s| vec![s]),
        Data::String(_) => Err(PromqlError::BadResponse(
            "the query returned a string, not a series".into(),
        )),
    }
}

fn series(
    labels: BTreeMap<String, String>,
    samples: impl IntoIterator<Item = RawSample>,
) -> Result<PromqlSeries, PromqlError> {
    let points = samples
        .into_iter()
        .map(|RawSample(ts, value)| {
            Ok(PromqlValue {
                ts: timestamp(ts)?,
                value: sample_value(&value)?,
            })
        })
        .collect::<Result<Vec<_>, PromqlError>>()?;
    Ok(PromqlSeries { labels, points })
}

/// Unix seconds (fractional) to a timestamp, to the millisecond Prometheus resolves.
fn timestamp(seconds: f64) -> Result<Timestamp, PromqlError> {
    if !seconds.is_finite() {
        return Err(PromqlError::BadResponse(format!(
            "invalid timestamp {seconds}"
        )));
    }
    Timestamp::from_millisecond((seconds * 1000.0).round() as i64)
        .map_err(|_| PromqlError::BadResponse(format!("timestamp {seconds} is out of range")))
}

/// A sample value: a Go-formatted float, including `NaN`, `+Inf` and `-Inf`.
fn sample_value(text: &str) -> Result<f64, PromqlError> {
    text.parse::<f64>()
        .map_err(|_| PromqlError::BadResponse(format!("`{text}` is not a number")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(body: &str) -> Vec<PromqlSeries> {
        match parse(body.as_bytes()).unwrap() {
            Reply::Success(series) => series,
            Reply::Failure { .. } => panic!("expected success"),
        }
    }

    #[test]
    fn vector_matrix_and_scalar() {
        let v = ok(
            r#"{"status":"success","data":{"resultType":"vector","result":[
            {"metric":{"b":"2","a":"1"},"value":[1700000000.5,"3.5"]}]}}"#,
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].labels.keys().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(v[0].points[0].ts.as_millisecond(), 1_700_000_000_500);
        assert_eq!(v[0].points[0].value, 3.5);

        let m = ok(
            r#"{"status":"success","data":{"resultType":"matrix","result":[
            {"metric":{},"values":[[1,"1"],[2,"2"],[3,"3"]]}]}}"#,
        );
        assert_eq!(
            m[0].points.iter().map(|p| p.value).collect::<Vec<_>>(),
            [1.0, 2.0, 3.0]
        );

        let s = ok(r#"{"status":"success","data":{"resultType":"scalar","result":[10,"2"]}}"#);
        assert_eq!(s.len(), 1);
        assert!(s[0].labels.is_empty());
        assert_eq!(s[0].points[0].value, 2.0);
    }

    #[test]
    fn special_floats_and_empty_results() {
        let v = ok(
            r#"{"status":"success","data":{"resultType":"vector","result":[
            {"metric":{"k":"nan"},"value":[1,"NaN"]},
            {"metric":{"k":"inf"},"value":[1,"+Inf"]},
            {"metric":{"k":"ninf"},"value":[1,"-Inf"]}]}}"#,
        );
        assert!(v[0].points[0].value.is_nan());
        assert_eq!(v[1].points[0].value, f64::INFINITY);
        assert_eq!(v[2].points[0].value, f64::NEG_INFINITY);
        assert!(
            ok(r#"{"status":"success","data":{"resultType":"vector","result":[]}}"#).is_empty()
        );
        assert!(
            ok(r#"{"status":"success","data":{"resultType":"matrix","result":[]}}"#).is_empty()
        );
    }

    #[test]
    fn extra_fields_like_stats_and_warnings_are_ignored() {
        let v = ok(
            r#"{"status":"success","warnings":["w"],"data":{"resultType":"vector","result":[]},"stats":{"seriesFetched":"0"}}"#,
        );
        assert!(v.is_empty());
    }

    #[test]
    fn error_envelopes_keep_type_and_message() {
        let Reply::Failure {
            error_type,
            message,
        } = parse(br#"{"status":"error","errorType":"bad_data","error":"parse error"}"#).unwrap()
        else {
            panic!("expected failure")
        };
        assert_eq!(
            (error_type.as_str(), message.as_str()),
            ("bad_data", "parse error")
        );
    }

    #[test]
    fn non_prometheus_bodies_are_bad_responses() {
        for body in [
            "<html>login</html>",
            "",
            r#"{"status":"success"}"#,
            r#"{"status":"success","data":{"resultType":"histogram","result":[]}}"#,
            r#"{"status":"success","data":{"resultType":"string","result":[1,"x"]}}"#,
            r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{},"value":[1,"abc"]}]}}"#,
            r#"{"status":"weird"}"#,
        ] {
            assert!(
                matches!(parse(body.as_bytes()), Err(PromqlError::BadResponse(_))),
                "{body}"
            );
        }
    }
}
