//! The metrics page as a workspace item.

use gpui::{App, SharedString};
use oxikube_ui::IconName;
use oxikube_workspace::{Item, TabContent};

use super::MetricsSettingsView;

/// The deduplication key of the tab: opening the page twice in a cluster's tab shows the open
/// one. (Each cluster tab has its own workspace, so the key needs no cluster in it.)
pub const ITEM_KEY: &str = "cluster-metrics-settings";

impl Item for MetricsSettingsView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Metrics").icon(IconName::ChartLine)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }
}
