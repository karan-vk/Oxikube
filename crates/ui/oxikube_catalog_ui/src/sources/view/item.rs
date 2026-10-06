//! The sources screen as a workspace item.

use gpui::{App, SharedString};
use oxikube_ui::IconName;
use oxikube_workspace::{Item, TabContent};

use super::SourcesView;

/// The deduplication key of the tab: opening it twice shows the open one.
const ITEM_KEY: &str = crate::sources::SOURCES_VIEW;

impl Item for SourcesView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Kubeconfig sources").icon(IconName::FileCode)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }
}
