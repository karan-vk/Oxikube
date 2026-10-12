//! What a fetched chart holds: plain series and reference lines in the metric's base unit
//! (cores, bytes), no GPUI and no chart types. The view converts these to its chart's points.

use std::collections::BTreeSet;
use std::sync::Arc;

use oxikube_domain::MetricUnit;
use oxikube_ports::PromqlSeries;

use super::charts::ChartDef;

/// Most reference lines one source adds to a chart; a namespace's containers do not each get a
/// line when they differ.
const MAX_REFERENCE_LINES: usize = 6;

/// One sample: seconds since the Unix epoch and the value (cores, bytes, bytes per second or a
/// count). A non-finite value is a gap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Seconds since the epoch.
    pub t: f64,
    /// The value.
    pub v: f64,
}

/// One line of a chart.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesData {
    /// The legend name: the container, the pod, or the measure.
    pub name: String,
    /// Ascending by time.
    pub samples: Arc<[Sample]>,
}

impl SeriesData {
    /// Whether the series has at least one reading.
    pub fn has_data(&self) -> bool {
        self.samples.iter().any(|s| s.v.is_finite())
    }
}

/// A horizontal line: a request, a limit or a capacity.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceLine {
    /// `requests`, `limits`, `capacity`, or `<container> requests` when the values differ.
    pub label: String,
    /// The value, in the chart's unit.
    pub value: f64,
    /// Which source made it.
    pub kind: ReferenceKind,
}

/// Which reference a line comes from, for its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    /// A request (or a capacity).
    Soft,
    /// A limit.
    Hard,
}

/// A chart's data for one range.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartData {
    /// The chart's id (`cpu`).
    pub id: &'static str,
    /// The chart's heading.
    pub title: &'static str,
    /// The unit of the values.
    pub unit: MetricUnit,
    /// The measured lines.
    pub series: Vec<SeriesData>,
    /// The request, limit and capacity lines.
    pub references: Vec<ReferenceLine>,
    /// Start of the range, seconds since the epoch.
    pub from: f64,
    /// End of the range, seconds since the epoch.
    pub to: f64,
}

impl ChartData {
    /// Whether no series has a reading (the endpoint answered with nothing for this object).
    pub fn is_empty(&self) -> bool {
        !self.series.iter().any(SeriesData::has_data)
    }
}

/// Why a chart could not be fetched, written to follow `metrics unavailable: `.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryError {
    /// What went wrong, with no credential in it.
    pub reason: String,
    /// Whether asking again may work (a timeout, a throttled server).
    pub retryable: bool,
}

impl HistoryError {
    /// `metrics unavailable: <reason>`.
    pub fn text(&self) -> String {
        format!("metrics unavailable: {}", self.reason)
    }
}

/// The series of one measure's result as chart lines. A result with one series is named by
/// `fallback` when the query names none; with several, by the series label (`pod`, `container`),
/// prefixed by the measure when the chart has more than one.
pub(super) fn lines_of(
    result: &[PromqlSeries],
    series_label: Option<&str>,
    fallback: &str,
    several_measures: bool,
) -> Vec<SeriesData> {
    result
        .iter()
        .map(|series| {
            let named = series_label
                .and_then(|label| series.labels.get(label))
                .filter(|name| !name.is_empty());
            let name = match (named, several_measures) {
                (Some(name), true) => format!("{name} {fallback}"),
                (Some(name), false) => name.clone(),
                (None, _) => fallback.to_owned(),
            };
            SeriesData {
                name,
                samples: series
                    .points
                    .iter()
                    .map(|p| Sample {
                        t: p.ts.as_millisecond() as f64 / 1000.0,
                        v: p.value,
                    })
                    .collect(),
            }
        })
        .collect()
}

/// The reference lines of one source's result: the last reading of each series. Series that are
/// zero or have no reading add no line (a container without a limit has none). When every series
/// has the same value (a workload's pods) one line carries `label`; when they differ each is
/// named after its series.
pub(super) fn reference_lines(
    result: &[PromqlSeries],
    series_label: Option<&str>,
    label: &str,
    kind: ReferenceKind,
) -> Vec<ReferenceLine> {
    let mut readings: Vec<(Option<String>, f64)> = result
        .iter()
        .filter_map(|series| {
            let value = series
                .points
                .iter()
                .rev()
                .map(|p| p.value)
                .find(|v| v.is_finite())?;
            (value > 0.0).then(|| {
                let name = series_label
                    .and_then(|l| series.labels.get(l))
                    .filter(|n| !n.is_empty())
                    .cloned();
                (name, value)
            })
        })
        .collect();
    let first = match readings.first() {
        Some((_, value)) => *value,
        None => return Vec::new(),
    };
    let same = |a: f64, b: f64| (a - b).abs() <= b.abs() * 1e-9;
    if readings.iter().all(|(_, v)| same(*v, first)) {
        return vec![ReferenceLine {
            label: label.to_owned(),
            value: first,
            kind,
        }];
    }
    readings.sort_by(|a, b| a.0.cmp(&b.0));
    let mut seen = BTreeSet::new();
    readings
        .into_iter()
        .filter(|(name, value)| seen.insert((name.clone(), value.to_bits())))
        .take(MAX_REFERENCE_LINES)
        .map(|(name, value)| ReferenceLine {
            label: match name {
                Some(name) => format!("{name} {label}"),
                None => label.to_owned(),
            },
            value,
            kind,
        })
        .collect()
}

/// Which kind of line a reference label is.
pub(super) fn reference_kind(label: &str) -> ReferenceKind {
    if label == "limits" {
        ReferenceKind::Hard
    } else {
        ReferenceKind::Soft
    }
}

impl ChartDef {
    /// An empty result of this chart over `from..to` (a chart whose queries all failed has none).
    pub(super) fn empty_data(&self, from: f64, to: f64) -> ChartData {
        ChartData {
            id: self.id,
            title: self.title,
            unit: self.unit,
            series: Vec::new(),
            references: Vec::new(),
            from,
            to,
        }
    }
}
