//! [`ContainerUsageCells`]: the usage cells of one container row (a pod's containers sub-table),
//! k9s' container view: CPU, MEM, `%CPU/R`, `%CPU/L`, `%MEM/R`, `%MEM/L`.

use oxikube_domain::metrics::ContainerUsage;
use oxikube_domain::view::ResourceAmounts;

use super::usage::{cpu_text, memory_text};
use crate::columns::pods::ABSENT;
use crate::columns::{Cell, CellSort, Tone, UsageThresholds};
use oxikube_domain::Quantity;

/// The usage cells of one container, in k9s' order. `–` without a reading (an init container,
/// a container not running, no metrics) and in a `%` cell without its request or limit.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerUsageCells {
    /// CPU in millicores.
    pub cpu: Cell<'static>,
    /// Memory in mebibytes.
    pub memory: Cell<'static>,
    /// CPU over the container's request.
    pub cpu_of_request: Cell<'static>,
    /// CPU over its limit, toned by the CPU threshold.
    pub cpu_of_limit: Cell<'static>,
    /// Memory over its request.
    pub memory_of_request: Cell<'static>,
    /// Memory over its limit, toned by the memory threshold.
    pub memory_of_limit: Cell<'static>,
}

impl ContainerUsageCells {
    /// The cells of a container with `usage` (its reading, if any), its `requests` and `limits`.
    pub fn new(
        usage: Option<&ContainerUsage>,
        requests: &ResourceAmounts,
        limits: &ResourceAmounts,
        thresholds: &UsageThresholds,
    ) -> Self {
        let cpu = usage
            .and_then(|u| u.cpu.value())
            .map(|n| Quantity::from_nanos(i128::from(n)));
        let memory = usage
            .and_then(|u| u.memory.value())
            .map(|b| Quantity::from_nanos(i128::from(b) * 1_000_000_000));
        let usage_cell =
            |value: Option<u64>, text: fn(u64) -> String, q: Option<Quantity>| match (value, q) {
                (Some(v), Some(q)) => Cell::quantity(text(v), q),
                _ => absent(),
            };
        Self {
            cpu: usage_cell(usage.and_then(|u| u.cpu.value()), cpu_text, cpu),
            memory: usage_cell(usage.and_then(|u| u.memory.value()), memory_text, memory),
            cpu_of_request: percent(cpu, requests.cpu, None),
            cpu_of_limit: percent(cpu, limits.cpu, Some(thresholds.cpu)),
            memory_of_request: percent(memory, requests.memory, None),
            memory_of_limit: percent(memory, limits.memory, Some(thresholds.memory)),
        }
    }

    /// The cells in k9s' order: CPU, MEM, `%CPU/R`, `%CPU/L`, `%MEM/R`, `%MEM/L`.
    pub fn cells(&self) -> [&Cell<'static>; 6] {
        [
            &self.cpu,
            &self.memory,
            &self.cpu_of_request,
            &self.cpu_of_limit,
            &self.memory_of_request,
            &self.memory_of_limit,
        ]
    }
}

fn absent() -> Cell<'static> {
    Cell::shown(ABSENT, CellSort::None)
}

/// `used` over `of` in whole percent; toned by `threshold` when given (a limit).
fn percent(
    used: Option<Quantity>,
    of: Option<Quantity>,
    threshold: Option<crate::columns::Threshold>,
) -> Cell<'static> {
    let Some(pct) = used.zip(of).and_then(|(used, of)| used.percent_of(&of)) else {
        return absent();
    };
    let tone = threshold.map_or(Tone::Neutral, |t| t.tone(pct));
    Cell::float(format!("{pct:.0}"), pct).with_tone(tone)
}
