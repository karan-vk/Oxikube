//! [`LiveMetricsConfig`]: a cluster's polling interval and thresholds, and
//! [`LiveMetricsConfigTable`], every cluster's (the `metrics` setting, pushed by the binary).

use std::collections::HashMap;
use std::time::Duration;

use oxikube_domain::ids::ClusterId;

use crate::columns::UsageThresholds;

/// The default polling interval: k9s' and Lens' 15 seconds (metrics-server scrapes every 15 s).
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(15);
/// The shortest interval a setting can ask for.
pub const MIN_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// The longest interval a setting can ask for.
pub const MAX_POLL_INTERVAL: Duration = Duration::from_secs(600);

/// One cluster's live metrics settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveMetricsConfig {
    /// How often a watched kind is polled while things go well.
    pub poll_interval: Duration,
    /// When a utilisation percent turns amber or red.
    pub thresholds: UsageThresholds,
}

impl Default for LiveMetricsConfig {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
            thresholds: UsageThresholds::default(),
        }
    }
}

impl LiveMetricsConfig {
    /// The same config with the interval kept within [`MIN_POLL_INTERVAL`] and
    /// [`MAX_POLL_INTERVAL`].
    #[must_use]
    pub fn clamped(mut self) -> Self {
        self.poll_interval = self
            .poll_interval
            .clamp(MIN_POLL_INTERVAL, MAX_POLL_INTERVAL);
        self
    }
}

/// Every cluster's [`LiveMetricsConfig`]: a default and the clusters that override it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveMetricsConfigTable {
    default: LiveMetricsConfig,
    clusters: HashMap<ClusterId, LiveMetricsConfig>,
}

impl LiveMetricsConfigTable {
    /// A table where every cluster uses `default`.
    pub fn new(default: LiveMetricsConfig) -> Self {
        Self {
            default: default.clamped(),
            clusters: HashMap::new(),
        }
    }

    /// The same table with `cluster` using `config`.
    #[must_use]
    pub fn with_cluster(mut self, cluster: ClusterId, config: LiveMetricsConfig) -> Self {
        self.clusters.insert(cluster, config.clamped());
        self
    }

    /// The config of `cluster`.
    pub fn get(&self, cluster: &ClusterId) -> LiveMetricsConfig {
        self.clusters.get(cluster).copied().unwrap_or(self.default)
    }
}
