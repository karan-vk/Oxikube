//! Live usage for the allocation views: one call to the metrics port, or the reason there is no
//! reading.

use std::sync::Arc;

use oxikube_domain::ids::ClusterId;
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, Quantity, QuantityFormat};
use oxikube_ports::{MetricsOutcome, MetricsPort};

use super::Usage;

/// The prefix every absence of metrics is shown with.
const PREFIX: &str = "metrics unavailable: ";

/// What a usage reading reads, or why there is none.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UsageState {
    /// The first reading has not arrived.
    #[default]
    Pending,
    /// A reading.
    Ready(MeasuredUsage),
    /// No reading, and why: the whole `metrics unavailable: <reason>` text.
    Unavailable(Arc<str>),
}

/// CPU and memory usage of a node, or summed over the pods of a namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasuredUsage {
    /// CPU in cores, or why it could not be read.
    pub cpu: Result<Quantity, MissingReason>,
    /// Working-set memory in bytes, or why it could not be read.
    pub memory: Result<Quantity, MissingReason>,
    /// For a namespace: pods with both readings. Zero for a node.
    pub pods_reporting: u32,
    /// For a namespace: pods whose sample lacks a reading of either resource. They are left out of
    /// that resource's sum, never counted as zero usage. Zero for a node.
    pub pods_unreadable: u32,
}

impl UsageState {
    /// The state for a metrics API that cannot answer, with the reason in words.
    pub fn unavailable(reason: MissingReason) -> Self {
        Self::Unavailable(unavailable_text(reason))
    }

    /// The state for a call that failed (network, timeout, expired credentials).
    pub fn failed(error: &OxiError) -> Self {
        Self::Unavailable(format!("{PREFIX}the request failed ({})", error.message()).into())
    }

    /// The CPU row's usage.
    pub(super) fn cpu(&self) -> Usage {
        self.row(|m| m.cpu)
    }

    /// The memory row's usage.
    pub(super) fn memory(&self) -> Usage {
        self.row(|m| m.memory)
    }

    fn row(&self, pick: impl Fn(&MeasuredUsage) -> Result<Quantity, MissingReason>) -> Usage {
        match self {
            Self::Pending => Usage::Pending,
            Self::Unavailable(text) => Usage::Unavailable(text.clone()),
            Self::Ready(measured) => match pick(measured) {
                Ok(value) => Usage::Value(value),
                Err(reason) => Usage::Unavailable(unavailable_text(reason)),
            },
        }
    }
}

/// The whole `metrics unavailable: <reason>` text for `reason`.
fn unavailable_text(reason: MissingReason) -> Arc<str> {
    format!("{PREFIX}{}", reason_text(reason)).into()
}

/// A reason as words, to follow `metrics unavailable: `.
fn reason_text(reason: MissingReason) -> &'static str {
    match reason {
        MissingReason::NotInstalled => "metrics-server is not installed in this cluster",
        MissingReason::ApiUnavailable => {
            "the metrics API is not answering (metrics-server may be starting or failing)"
        }
        MissingReason::NotYetAvailable => "no reading yet (metrics-server has not scraped it)",
        MissingReason::Forbidden => "you are not allowed to read the metrics API",
        MissingReason::Other => "the reading is missing or unreadable",
    }
}

fn cpu_of(sample: &MetricsSample) -> Result<Quantity, MissingReason> {
    sample
        .cpu_quantity()
        .ok_or_else(|| sample.cpu.missing().unwrap_or(MissingReason::Other))
}

fn memory_of(sample: &MetricsSample) -> Result<Quantity, MissingReason> {
    sample
        .memory_quantity()
        .ok_or_else(|| sample.memory.missing().unwrap_or(MissingReason::Other))
}

/// The usage of `node` from one `node_metrics` call. Runs off the UI thread (a network call).
///
/// The port answers for every node at once (`metrics.k8s.io` lists them in one request), so this
/// asks for all of them and keeps one; the cluster-wide polling service (E13-S03) will share one
/// poll between every view.
pub async fn node_usage(port: &dyn MetricsPort, cluster: &ClusterId, node: &str) -> UsageState {
    match port.node_metrics(cluster).await {
        Err(error) => UsageState::failed(&error),
        Ok(MetricsOutcome::Unavailable(reason)) => UsageState::unavailable(reason),
        Ok(MetricsOutcome::Available(samples)) => samples
            .iter()
            .find(|s| matches!(&s.subject, MetricsSubject::Node { node: n } if &**n == node))
            .map_or(
                UsageState::unavailable(MissingReason::NotYetAvailable),
                |sample| {
                    UsageState::Ready(MeasuredUsage {
                        cpu: cpu_of(sample),
                        memory: memory_of(sample),
                        pods_reporting: 0,
                        pods_unreadable: 0,
                    })
                },
            ),
    }
}

/// The usage of the pods of `namespace`, summed, from one `pod_metrics` call for that namespace
/// alone. A pod whose sample lacks a reading is counted in
/// [`pods_unreadable`](MeasuredUsage::pods_unreadable), never as zero. Runs off the UI thread.
pub async fn namespace_usage(
    port: &dyn MetricsPort,
    cluster: &ClusterId,
    namespace: &str,
) -> UsageState {
    let selection = NamespaceSelection::from_names([namespace]);
    match port.pod_metrics(cluster, &selection).await {
        Err(error) => UsageState::failed(&error),
        Ok(MetricsOutcome::Unavailable(reason)) => UsageState::unavailable(reason),
        Ok(MetricsOutcome::Available(samples)) => sum_pods(&samples, namespace),
    }
}

fn sum_pods(samples: &[MetricsSample], namespace: &str) -> UsageState {
    let mut cpu = Tally::default();
    let mut memory = Tally::default();
    let (mut reporting, mut unreadable) = (0u32, 0u32);
    for sample in samples {
        let MetricsSubject::Pod { pod } = &sample.subject else {
            continue;
        };
        if pod.namespace.as_deref() != Some(namespace) {
            continue;
        }
        let (c, m) = (cpu_of(sample), memory_of(sample));
        if c.is_ok() && m.is_ok() {
            reporting += 1;
        } else {
            unreadable += 1;
        }
        cpu.add(c);
        memory.add(m);
    }
    UsageState::Ready(MeasuredUsage {
        cpu: cpu.finish(None),
        memory: memory.finish(Some(QuantityFormat::BinarySI)),
        pods_reporting: reporting,
        pods_unreadable: unreadable,
    })
}

/// The sum of one resource over the pods that report it, and why the others do not. CPU and
/// memory are tallied on their own: a pod missing one still counts for the other.
#[derive(Default)]
struct Tally {
    nanos: i128,
    reported: u32,
    missing: Option<MissingReason>,
}

impl Tally {
    fn add(&mut self, reading: Result<Quantity, MissingReason>) {
        match reading {
            Ok(value) => {
                self.nanos = self.nanos.saturating_add(value.nanos());
                self.reported += 1;
            }
            Err(reason) => {
                self.missing.get_or_insert(reason);
            }
        }
    }

    /// The sum (printed in `format`, when given); or, when pods exist but none reports, the
    /// reason.
    fn finish(self, format: Option<QuantityFormat>) -> Result<Quantity, MissingReason> {
        if self.reported == 0
            && let Some(reason) = self.missing
        {
            return Err(reason);
        }
        let sum = Quantity::from_nanos(self.nanos);
        Ok(format.map_or(sum, |format| sum.with_format(format)))
    }
}
