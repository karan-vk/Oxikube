//! The catalog as a workspace item: the first tab of the window.

use gpui::{App, SharedString};
use oxikube_ui::IconName;
use oxikube_workspace::{Item, TabContent};

use super::CatalogView;

/// The deduplication key of the catalog tab: opening it twice shows the open one.
const ITEM_KEY: &str = "catalog";

impl Item for CatalogView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Clusters").icon(IconName::Boxes)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }
}
