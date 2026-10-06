//! The `resource_table` settings: whether a table remembers its filter.

use oxikube_settings::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What one settings layer says about resource tables: the `resource_table` object of
/// `settings.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ResourceTableContent {
    /// Remember the filter typed in each kind's table (the `/` bar) and restore it the next time
    /// that table opens, like Freelens' persistent search. Off by default: a filter you forgot
    /// about is a table that looks empty. The filter text is saved per kind (not per cluster).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persist_filter: Option<bool>,
}

/// The resolved `resource_table` settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceTableSettings {
    /// Whether each table's filter is saved and restored.
    pub persist_filter: bool,
}

impl Settings for ResourceTableSettings {
    const KEY: Option<&'static str> = Some("resource_table");
    type Content = ResourceTableContent;

    fn from_content(content: ResourceTableContent) -> Self {
        Self {
            persist_filter: content.persist_filter.unwrap_or(false),
        }
    }
}

oxikube_settings::register_settings!(ResourceTableSettings);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_the_filter_is_off_unless_the_setting_says_so() {
        assert!(
            !ResourceTableSettings::from_content(ResourceTableContent::default()).persist_filter
        );
        let on = ResourceTableContent {
            persist_filter: Some(true),
        };
        assert!(ResourceTableSettings::from_content(on).persist_filter);
    }
}
