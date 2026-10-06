//! How a restore connects: which clusters at once, how many in parallel, how long to wait.

use std::time::Duration;

/// Which of the restored clusters connect at launch (the `session.restore_connect` setting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RestoreConnect {
    /// Only the cluster whose tab was displayed. The others connect when their tab is first
    /// shown ("sessions lazy", the epic's risk mitigation). The default.
    #[default]
    Active,
    /// Every restored cluster, the displayed one first, a few at a time
    /// ([`RestoreConfig::concurrency`]).
    All,
}

/// Limits on a restore's connects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestoreConfig {
    /// How many clusters connect at the same time under [`RestoreConnect::All`] (at least 1).
    /// Slow clusters hold only their own slot.
    pub concurrency: usize,
    /// How long one cluster may take to connect, retries included. Past it the attempt is
    /// cancelled and the cluster's tab shows the timeout as its error.
    pub connect_timeout: Duration,
}

impl Default for RestoreConfig {
    fn default() -> Self {
        Self {
            concurrency: 2,
            connect_timeout: Duration::from_secs(30),
        }
    }
}
