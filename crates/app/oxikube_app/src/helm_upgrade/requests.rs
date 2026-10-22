//! [`UpgradeRequests`]: the hand-off from `helm::UpgradeRelease` to the upgrade tab.

use oxikube_domain::helm::ChartRef;
use tokio::sync::watch;

/// What `helm::UpgradeRelease` asked for: a release to upgrade, or none (the tab lists the
/// cluster's releases to pick from).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeTarget {
    /// The release's namespace and name, when given.
    pub release: Option<(String, String)>,
    /// A chart to prefer as the source (set by the charts catalog's "upgrade to this version"
    /// later); `None` for the chart the release runs.
    pub chart: Option<ChartRef>,
}

/// Carries `helm::UpgradeRelease` requests to the window that shows the upgrade tab. Cheap to
/// clone; clones share the channel.
#[derive(Debug, Clone)]
pub struct UpgradeRequests {
    tx: watch::Sender<Option<UpgradeTarget>>,
}

impl Default for UpgradeRequests {
    fn default() -> Self {
        Self::new()
    }
}

impl UpgradeRequests {
    /// A channel with no request yet.
    pub fn new() -> Self {
        Self {
            tx: watch::channel(None).0,
        }
    }

    /// Asks the window to open the upgrade tab for `target`. Nothing is upgraded.
    pub fn request(&self, target: UpgradeTarget) {
        self.tx.send_replace(Some(target));
    }

    /// The requests as they come (the current value is the latest one made before subscribing).
    pub fn subscribe(&self) -> watch::Receiver<Option<UpgradeTarget>> {
        self.tx.subscribe()
    }
}
