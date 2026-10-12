//! The live-metrics keys of the `metrics` setting (E13-S03): `poll_interval_secs` and
//! `thresholds`, the k9s-style warn and critical percents.
//!
//! ```json
//! "metrics": {
//!   "poll_interval_secs": 15,
//!   "thresholds": { "cpu": { "warn": 70, "critical": 90 }, "memory": { "warn": 70, "critical": 90 } }
//! }
//! ```

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The default interval between two polls of metrics-server, in seconds.
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 15;
/// The shortest interval accepted; a smaller one is raised to it.
pub const MIN_POLL_INTERVAL_SECS: u64 = 5;
/// The longest interval accepted; a larger one is lowered to it.
pub const MAX_POLL_INTERVAL_SECS: u64 = 600;
/// k9s' default warn level, in percent.
pub const DEFAULT_WARN_PERCENT: u32 = 70;
/// k9s' default critical level, in percent.
pub const DEFAULT_CRITICAL_PERCENT: u32 = 90;
/// The highest level accepted (a pod may use many times its request).
const MAX_PERCENT: u32 = 1000;

/// What one layer says about a resource's levels.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageLevelContent {
    /// From this percent of the limit (or of a node's allocatable) on, the cell is amber.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn: Option<u32>,
    /// From this percent on, the cell is red.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical: Option<u32>,
}

/// What one layer says about `metrics.thresholds`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ThresholdsContent {
    /// CPU usage over a pod's or container's CPU limit, or over a node's allocatable CPU.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<UsageLevelContent>,
    /// Memory usage over its limit, or over a node's allocatable memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<UsageLevelContent>,
}

/// The resolved warn and critical levels of one resource, in whole percent. `warn` is never
/// above `critical`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageLevel {
    /// Amber from here.
    pub warn: u32,
    /// Red from here.
    pub critical: u32,
}

impl Default for UsageLevel {
    fn default() -> Self {
        Self {
            warn: DEFAULT_WARN_PERCENT,
            critical: DEFAULT_CRITICAL_PERCENT,
        }
    }
}

impl UsageLevel {
    /// The levels `content` sets, the defaults for what it leaves out, kept in range.
    pub fn of(content: Option<&UsageLevelContent>) -> Self {
        let content = content.cloned().unwrap_or_default();
        let critical = content
            .critical
            .unwrap_or(DEFAULT_CRITICAL_PERCENT)
            .clamp(1, MAX_PERCENT);
        let warn = content.warn.unwrap_or(DEFAULT_WARN_PERCENT).min(critical);
        Self { warn, critical }
    }
}

/// The poll interval `secs` asks for, kept within [`MIN_POLL_INTERVAL_SECS`] and
/// [`MAX_POLL_INTERVAL_SECS`].
pub fn poll_interval(secs: Option<u64>) -> u64 {
    secs.unwrap_or(DEFAULT_POLL_INTERVAL_SECS)
        .clamp(MIN_POLL_INTERVAL_SECS, MAX_POLL_INTERVAL_SECS)
}
