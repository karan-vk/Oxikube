//! What `top_pods` and `top_nodes` share: the sort key and limit, ranking rows, and the figures'
//! text.

use std::cmp::Ordering;

use jiff::Timestamp;
use oxikube_domain::metrics::{MetricsSample, MissingReason};
use oxikube_domain::{OxiError, OxiResult};
use serde_json::{Value, json};

use super::bounds::{DEFAULT_LIMIT, MAX_LIMIT};
use crate::metrics::live::{cpu_text, memory_text, reason_text};

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
    pub(super) cpu_nanocores: Result<u64, MissingReason>,
    pub(super) memory_bytes: Result<u64, MissingReason>,
    pub(super) at: Timestamp,
    /// Usage as a percent of what the subject offers, when that is known (nodes).
    pub(super) cpu_percent: Option<f64>,
    pub(super) memory_percent: Option<f64>,
}

impl Row {
    pub(super) fn of(sample: &MetricsSample, namespace: Option<&str>, name: &str) -> Self {
        let reading = |r: oxikube_domain::metrics::Reading| match r {
            oxikube_domain::metrics::Reading::Value(v) => Ok(v),
            oxikube_domain::metrics::Reading::Missing(why) => Err(why),
        };
        Self {
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
            cpu_nanocores: reading(sample.cpu),
            memory_bytes: reading(sample.memory),
            at: sample.ts,
            cpu_percent: None,
            memory_percent: None,
        }
    }

    fn key(&self, sort: SortKey) -> Option<u64> {
        match sort {
            SortKey::Cpu => self.cpu_nanocores.ok(),
            SortKey::Memory => self.memory_bytes.ok(),
        }
    }

    /// `250m`, or the reason there is no reading.
    pub(super) fn cpu_text(&self) -> String {
        match self.cpu_nanocores {
            Ok(n) => cpu_text(n),
            Err(why) => format!("n/a ({})", reason_text(why)),
        }
    }

    /// `128Mi`, or the reason there is no reading.
    pub(super) fn memory_text(&self) -> String {
        match self.memory_bytes {
            Ok(n) => memory_text(n),
            Err(why) => format!("n/a ({})", reason_text(why)),
        }
    }

    /// The row's figures as JSON: millicores and bytes as numbers, `null` for no reading.
    pub(super) fn figures(&self) -> Value {
        json!({
            "cpu_millicores": self.cpu_nanocores.ok().map(|n| n / 1_000_000),
            "cpu": self.cpu_text(),
            "memory_bytes": self.memory_bytes.ok(),
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
