//! [`UsageThresholds`]: when a utilisation percent turns amber or red (E13-S03).
//!
//! The same two numbers as k9s' `thresholds.cpu` and `thresholds.memory` (warn 70, critical 90
//! by default), set by the `metrics.thresholds` setting globally or per cluster. They colour the
//! columns that compare usage with a ceiling: a pod's and a container's `%CPU/L` and `%MEM/L`,
//! and a node's `CPU%` and `Mem%` (usage over allocatable). The `/R` columns stay neutral: a pod
//! using more than it requested is bursting as its limit allows, not running out.
//!
//! A percent is compared as it is shown, rounded to a whole number, so a cell that reads `70`
//! is amber whatever the decimals behind it. Reaching a threshold counts: `70` with warn `70` is
//! amber and `90` with critical `90` is red.

use super::Tone;

/// k9s' default warn threshold, in percent.
pub const DEFAULT_WARN: u32 = 70;
/// k9s' default critical threshold, in percent.
pub const DEFAULT_CRITICAL: u32 = 90;

/// The warn and critical levels of one resource, in whole percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Threshold {
    /// From this percent on, a cell is [`Tone::Warn`].
    pub warn: u32,
    /// From this percent on, a cell is [`Tone::Error`].
    pub critical: u32,
}

impl Default for Threshold {
    fn default() -> Self {
        Self {
            warn: DEFAULT_WARN,
            critical: DEFAULT_CRITICAL,
        }
    }
}

impl Threshold {
    /// A threshold of `warn` and `critical` percent. A `warn` above `critical` is lowered to it,
    /// so the levels never cross.
    pub fn new(warn: u32, critical: u32) -> Self {
        Self {
            warn: warn.min(critical),
            critical,
        }
    }

    /// The tone of `percent`, compared as it is shown (rounded to a whole number).
    pub fn tone(&self, percent: f64) -> Tone {
        let shown = percent.round();
        if shown >= f64::from(self.critical) {
            Tone::Error
        } else if shown >= f64::from(self.warn) {
            Tone::Warn
        } else {
            Tone::Neutral
        }
    }
}

/// The thresholds of CPU and memory: the `metrics.thresholds` setting of one cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct UsageThresholds {
    /// CPU usage over its ceiling.
    pub cpu: Threshold,
    /// Memory usage over its ceiling.
    pub memory: Threshold,
}

impl UsageThresholds {
    /// The threshold of CPU (`cpu == true`) or memory.
    pub fn of(&self, cpu: bool) -> Threshold {
        if cpu { self.cpu } else { self.memory }
    }
}
