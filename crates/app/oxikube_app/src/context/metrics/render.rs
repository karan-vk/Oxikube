//! A pod's usage as the text of a [`ContextBlock`].

use std::fmt::Write as _;

use oxikube_domain::agent::{ContextBlock, MAX_CONTEXT_BLOCK_BYTES};
use oxikube_domain::metrics::{ContainerUsage, Reading};
use oxikube_domain::utilisation::{PodResources, Utilisation};

use crate::metrics::live::{UsagePoint, cpu_text, memory_text, reason_text};

/// Most samples quoted; the ring holds 60, a prompt needs the recent shape.
pub(super) const MAX_SAMPLES: usize = 20;

/// Bytes kept for the header, the requests and limits, and the containers.
const RESERVE: usize = 1_536;

/// Where a pod's usage came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    /// The ring buffer of a watched cluster: the history.
    History,
    /// One on-demand reading: nothing is watching this cluster, so there is no history.
    OneReading,
}

/// Everything the block says.
pub(super) struct PodMetrics<'a> {
    pub(super) cluster: &'a str,
    pub(super) cluster_id: &'a str,
    pub(super) namespace: &'a str,
    pub(super) pod: &'a str,
    /// Oldest first.
    pub(super) points: &'a [UsagePoint],
    pub(super) origin: Origin,
    /// `metrics unavailable: ...` when the last poll failed and the samples are older.
    pub(super) poll_note: Option<&'a str>,
    pub(super) containers: &'a [ContainerUsage],
    /// `Err` is why requests and limits are not quoted.
    pub(super) resources: Result<&'a PodResources, &'a str>,
}

fn reading(r: Reading, text: fn(u64) -> String) -> String {
    match r {
        Reading::Value(v) => text(v),
        Reading::Missing(why) => format!("n/a ({})", reason_text(why)),
    }
}

fn figure(value: Option<u64>, text: fn(u64) -> String) -> String {
    value.map_or_else(|| "n/a".to_owned(), text)
}

fn amount(q: &oxikube_domain::Quantity) -> String {
    if q.is_zero() {
        "none".to_owned()
    } else {
        q.to_string()
    }
}

fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_owned(), |p| format!("{p:.1}%"))
}

/// The block for `m`, within `cap` bytes of body: older samples are dropped first, and the block
/// says so.
pub(super) fn block(m: &PodMetrics<'_>, cap: usize) -> ContextBlock {
    let cap = cap.min(MAX_CONTEXT_BLOCK_BYTES);
    let mut head = String::new();
    let _ = writeln!(head, "# cluster: {} ({})", m.cluster, m.cluster_id);
    let _ = writeln!(head, "# pod: {}/{}", m.namespace, m.pod);
    let source = match m.origin {
        Origin::History => "metrics-server, as polled while the cluster is open",
        Origin::OneReading => {
            "metrics-server, one reading now (the cluster's metrics are not being watched, so there is no history)"
        }
    };
    let _ = writeln!(head, "# source: {source}");
    if let Some(note) = m.poll_note {
        let _ = writeln!(head, "# note: {note} (the samples below are older)");
    }
    if let (Some(first), Some(last)) = (m.points.first(), m.points.last()) {
        let _ = writeln!(
            head,
            "# samples: {} from {} to {}",
            m.points.len(),
            first.at,
            last.at
        );
    }
    match m.resources {
        Ok(resources) => {
            let _ = writeln!(
                head,
                "# requests: cpu {}, memory {}; limits: cpu {}, memory {}",
                amount(&resources.requests.cpu),
                amount(&resources.requests.memory),
                amount(&resources.limits.cpu),
                amount(&resources.limits.memory),
            );
            if let Some(last) = m.points.last() {
                let u = Utilisation::of(
                    last.cpu_quantity().as_ref(),
                    last.memory_quantity().as_ref(),
                    resources,
                );
                let _ = writeln!(
                    head,
                    "# now: cpu {} ({} of requests, {} of limits), memory {} ({} of requests, {} of limits)",
                    figure(last.cpu_nanocores, cpu_text),
                    percent(u.cpu_of_requests),
                    percent(u.cpu_of_limits),
                    figure(last.memory_bytes, memory_text),
                    percent(u.memory_of_requests),
                    percent(u.memory_of_limits),
                );
            }
        }
        Err(why) => {
            let _ = writeln!(head, "# requests and limits: not quoted ({why})");
        }
    }

    let mut tail = String::new();
    if !m.containers.is_empty() {
        let _ = writeln!(tail, "# containers (latest): name cpu memory");
        for c in m.containers {
            let _ = writeln!(
                tail,
                "{}\t{}\t{}",
                c.name,
                reading(c.cpu, cpu_text),
                reading(c.memory, memory_text)
            );
        }
    }

    // Newest samples first into the budget, so what is cut is the oldest.
    let room = cap.saturating_sub(head.len() + tail.len() + RESERVE.min(cap / 4));
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0;
    for point in m.points.iter().rev().take(MAX_SAMPLES) {
        let line = format!(
            "{}\t{}\t{}\n",
            point.at,
            figure(point.cpu_nanocores, cpu_text),
            figure(point.memory_bytes, memory_text)
        );
        if used + line.len() > room {
            break;
        }
        used += line.len();
        lines.push(line);
    }
    let left_out = m.points.len() - lines.len();
    lines.reverse();

    let mut body = head;
    if left_out > 0 {
        let why = if m.points.len() > MAX_SAMPLES && lines.len() == MAX_SAMPLES {
            format!("only the newest {MAX_SAMPLES} are quoted")
        } else {
            "the size budget".to_owned()
        };
        let _ = writeln!(body, "# note: {left_out} older samples left out ({why})");
    }
    if !lines.is_empty() {
        body.push_str("# time cpu memory (oldest first)\n");
        lines.iter().for_each(|l| body.push_str(l));
    }
    body.push_str(&tail);

    let title = format!(
        "Metrics pod {}/{} ({} samples)",
        m.namespace,
        m.pod,
        lines.len()
    );
    let mut block = ContextBlock::bounded(title, "text/plain", body);
    block.truncated |= left_out > 0;
    block
}

/// A block that says why there is nothing to quote: the pod does not exist, or metrics are
/// unavailable. The reason reads `metrics unavailable: ...` where metrics are the cause.
pub(super) fn error_block(namespace: &str, pod: &str, reason: &str) -> ContextBlock {
    ContextBlock::bounded(
        format!("Metrics pod {namespace}/{pod}: unavailable"),
        "text/plain",
        format!("# pod: {namespace}/{pod}\n# error: {reason}\n"),
    )
}
