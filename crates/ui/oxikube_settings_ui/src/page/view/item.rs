//! The settings page as a workspace item.

use gpui::{App, SharedString};
use oxikube_ui::IconName;
use oxikube_workspace::{Item, TabContent};

use super::SettingsPage;

/// The page's id in `view::Open` and its tab key: opening the settings again shows the open tab.
pub const SETTINGS_VIEW: &str = "settings";

impl Item for SettingsPage {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Settings").icon(IconName::Settings)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(SETTINGS_VIEW.into())
    }
}
