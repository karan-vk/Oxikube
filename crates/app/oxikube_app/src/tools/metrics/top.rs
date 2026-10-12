//! What `top_pods` and `top_nodes` share: the sort key and limit, ranking rows, and the figures'
//! text.

use std::cmp::Ordering;

use jiff::Timestamp;
use oxikube_domain::metrics::{MetricsSample, Reading};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{MetricsOutcome, ToolOutput};
use serde_json::{Value, json};

use super::bounds::{DEFAULT_LIMIT, MAX_LIMIT};
use super::unavailable::unavailable;
use crate::metrics::live::{cpu_text, memory_text, reading_text, reason_text};

/// What a `top_*` call ranks by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SortKey {
    Cpu,
    Memory,
}

impl SortKey {
    pub(super) const fn id(self) -> &'static str {
        match self {
            SortKey::Cpu => "cpu",
            SortKey::Memory => "memory",
        }
    }
}

/// The `sort` and `limit` arguments (the schema already checked their types and ranges; these
/// checks keep a direct call honest too).
pub(super) fn sort_and_limit(args: &Value) -> OxiResult<(SortKey, usize)> {
    let sort = match args.get("sort").and_then(Value::as_str) {
        None | Some("cpu") => SortKey::Cpu,
        Some("memory") => SortKey::Memory,
        Some(other) => {
            return Err(OxiError::validation(format!(
                "sort {other:?}: expected `cpu` or `memory`"
            )));
        }
    };
    let limit = match args.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(value) => value
            .as_u64()
            .filter(|n| (1..=MAX_LIMIT as u64).contains(n))
            .map(|n| n as usize)
            .ok_or_else(|| {
                OxiError::validation(format!("limit must be an integer from 1 to {MAX_LIMIT}"))
            })?,
    };
    Ok((sort, limit))
}

/// One ranked subject: a pod or a node with its readings.
#[derive(Debug, Clone)]
pub(super) struct Row {
    pub(super) namespace: Option<String>,
    pub(super) name: String,
    pub(super) cpu: Reading,
    pub(super) memory: Reading,
    pub(super) at: Timestamp,
    /// Usage as a percent of what the subject offers, when that is known (nodes).
    pub(super) cpu_percent: Option<f64>,
    pub(super) memory_percent: Option<f64>,
}

impl Row {
    pub(super) fn of(sample: &MetricsSample, namespace: Option<&str>, name: &str) -> Self {
        Self {
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
            cpu: sample.cpu,
            memory: sample.memory,
            at: sample.ts,
            cpu_percent: None,
            memory_percent: None,
        }
    }

    fn key(&self, sort: SortKey) -> Option<u64> {
        match sort {
            SortKey::Cpu => self.cpu.value(),
            SortKey::Memory => self.memory.value(),
        }
    }

    /// `250m`, or the reason there is no reading.
    pub(super) fn cpu_text(&self) -> String {
        reading_text(self.cpu, cpu_text)
    }

    /// `128Mi`, or the reason there is no reading.
    pub(super) fn memory_text(&self) -> String {
        reading_text(self.memory, memory_text)
    }

    /// The row's figures as JSON: millicores and bytes as numbers, `null` for no reading.
    pub(super) fn figures(&self) -> Value {
        json!({
            "cpu_millicores": self.cpu.value().map(|n| n / 1_000_000),
            "cpu": self.cpu_text(),
            "memory_bytes": self.memory.value(),
            "memory": self.memory_text(),
        })
    }
}

/// Orders `rows` by `sort`, biggest first, rows without a reading last, ties by name; the
/// result is deterministic for equal inputs.
pub(super) fn rank(rows: &mut [Row], sort: SortKey) {
    rows.sort_by(|a, b| {
        match (a.key(sort), b.key(sort)) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
        .then_with(|| a.namespace.cmp(&b.namespace))
        .then_with(|| a.name.cmp(&b.name))
    });
}

/// The newest sample time of `rows`.
pub(super) fn newest(rows: &[Row]) -> Option<Timestamp> {
    rows.iter().map(|row| row.at).max()
}

/// The samples of a snapshot answer, or the visible `unavailable` result for a call that timed
/// out (`None`), failed or found no metrics API.
pub(super) fn samples_or_unavailable(
    cluster: &str,
    answer: Option<OxiResult<MetricsOutcome>>,
) -> Result<Vec<MetricsSample>, ToolOutput> {
    match answer {
        None => Err(unavailable(
            cluster,
            "the metrics API did not answer in time",
        )),
        Some(Err(error)) => Err(unavailable(cluster, error.message())),
        Some(Ok(MetricsOutcome::Unavailable(why))) => Err(unavailable(cluster, reason_text(why))),
        Some(Ok(MetricsOutcome::Available(samples))) => Ok(samples),
    }
}
