//! A PromQL result as a bounded, rounded answer.

use oxikube_domain::metric_query::MetricUnit;
use oxikube_domain::redact::redact;
use oxikube_ports::{ContentPart, PromqlSeries, PromqlValue, ToolOutput};
use serde_json::{Value, json};

use crate::tools::metrics::bounds::{MAX_POINTS_PER_SERIES, MAX_SERIES};

/// Points across all series of one answer; with many series each gets fewer, so the answer stays
/// small whatever the query matched.
const MAX_TOTAL_POINTS: usize = 400;

/// What the answer says about where it came from.
pub(super) struct Source<'a> {
    pub(super) cluster: &'a str,
    /// The stack that answered, as the user knows it.
    pub(super) provider: &'a str,
    /// The PromQL that ran.
    pub(super) promql: &'a str,
    pub(super) unit: Option<MetricUnit>,
    /// The span of a range query.
    pub(super) range: Option<std::time::Duration>,
    pub(super) note: Option<&'a str>,
}

/// `value` rounded to four significant digits (a whole number from four integer digits up), or
/// `None` for NaN and infinities, which JSON cannot hold.
pub(super) fn round_value(value: f64) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    if value == 0.0 {
        return Some(0.0);
    }
    let magnitude = value.abs().log10().floor() as i32;
    let decimals = (3 - magnitude).clamp(0, 9);
    let factor = 10f64.powi(decimals);
    Some((value * factor).round() / factor)
}

/// At most `keep` of `points`, evenly spread, always keeping the first and the last.
pub(super) fn thin(points: &[PromqlValue], keep: usize) -> Vec<PromqlValue> {
    if points.len() <= keep || keep < 2 {
        return if keep < 2 {
            points.last().copied().into_iter().collect()
        } else {
            points.to_vec()
        };
    }
    let last = points.len() - 1;
    (0..keep).map(|i| points[i * last / (keep - 1)]).collect()
}

fn unit_id(unit: MetricUnit) -> &'static str {
    match unit {
        MetricUnit::Cores => "cores",
        MetricUnit::Bytes => "bytes",
        MetricUnit::BytesPerSecond => "bytes_per_second",
        MetricUnit::Count => "count",
    }
}

fn last_value(series: &PromqlSeries) -> f64 {
    series.points.last().map_or(f64::NEG_INFINITY, |p| {
        if p.value.is_nan() {
            f64::NEG_INFINITY
        } else {
            p.value
        }
    })
}

/// The answer for `series`: the biggest [`MAX_SERIES`] by their last value, each thinned.
pub(super) fn render(source: &Source<'_>, mut series: Vec<PromqlSeries>) -> ToolOutput {
    let total = series.len();
    series.sort_by(|a, b| {
        last_value(b)
            .total_cmp(&last_value(a))
            .then_with(|| a.labels.cmp(&b.labels))
    });
    series.truncate(MAX_SERIES);
    let per_series = (MAX_TOTAL_POINTS / series.len().max(1)).clamp(2, MAX_POINTS_PER_SERIES);
    let mut thinned = false;
    let listed: Vec<Value> = series
        .iter()
        .map(|s| {
            let points = thin(&s.points, per_series);
            thinned |= points.len() < s.points.len();
            let labels: serde_json::Map<String, Value> = s
                .labels
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(redact(v).into_owned())))
                .collect();
            let pairs: Vec<Value> = points
                .iter()
                .map(|p| json!([p.ts.as_second(), round_value(p.value)]))
                .collect();
            let mut entry = json!({"labels": labels, "points": pairs});
            if let Some(last) = s.points.last() {
                entry["latest"] = json!(round_value(last.value));
                entry["latest_at"] = json!(last.ts.to_string());
            }
            entry
        })
        .collect();

    let mut notes: Vec<String> = Vec::new();
    if total > listed.len() {
        notes.push(format!(
            "{} more series were left out (the {} with the highest latest value are shown)",
            total - listed.len(),
            listed.len()
        ));
    }
    if thinned {
        notes.push(format!(
            "points were thinned to at most {per_series} per series; each is [unix seconds, value]"
        ));
    }
    if let Some(note) = source.note {
        notes.push(note.to_owned());
    }
    if total == 0 {
        notes.push("the query matched no series".to_owned());
    }
    let value = json!({
        "status": "ok",
        "cluster": source.cluster,
        "provider": source.provider,
        "query": redact(source.promql),
        "type": if source.range.is_some() { "range" } else { "instant" },
        "range_seconds": source.range.map(|r| r.as_secs()),
        "unit": source.unit.map(unit_id),
        "series_total": total,
        "series": listed,
        "notes": notes,
    });
    ToolOutput {
        content: vec![ContentPart::text(value.to_string())],
        structured: Some(value),
        is_error: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(n: usize) -> Vec<PromqlValue> {
        (0..n)
            .map(|i| PromqlValue {
                ts: jiff::Timestamp::from_second(1_000 + i as i64).unwrap(),
                value: i as f64,
            })
            .collect()
    }

    #[test]
    fn values_keep_four_significant_digits_and_whole_numbers_stay_whole() {
        assert_eq!(round_value(0.123_456), Some(0.1235));
        assert_eq!(round_value(29.123_456), Some(29.12));
        assert_eq!(round_value(1_234.567), Some(1_235.0));
        assert_eq!(round_value(1_234_567_890.0), Some(1_234_567_890.0));
        assert_eq!(round_value(-0.000_012_345_6), Some(-0.000_012_35));
        assert_eq!(round_value(0.0), Some(0.0));
        assert_eq!(round_value(f64::NAN), None);
        assert_eq!(round_value(f64::INFINITY), None);
    }

    #[test]
    fn thinning_keeps_the_ends_and_is_even() {
        let all = points(100);
        let kept = thin(&all, 5);
        assert_eq!(kept.len(), 5);
        assert_eq!(kept[0], all[0]);
        assert_eq!(kept[4], all[99]);
        assert_eq!(thin(&all, 200), all, "short series are untouched");
        assert_eq!(thin(&all, 1), vec![all[99]], "one point is the latest");
        assert_eq!(thin(&all, 5), thin(&all, 5), "deterministic");
    }
}
