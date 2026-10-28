//! The Cloud section as a workspace item.

use gpui::{App, SharedString};
use oxikube_ui::IconName;
use oxikube_workspace::{Item, TabContent};

use super::CloudView;

impl Item for CloudView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Cloud clusters").icon(IconName::Cloud)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(crate::cloud::CLOUD_VIEW.into())
    }
}
