//! The `session` settings of session restore (E06-S11): whether to reopen the last session at
//! launch, and which clusters connect.

use oxikube_app::session::restore::RestoreConnect;
use oxikube_settings::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which restored clusters connect at launch (`session.restore_connect`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestoreConnectSetting {
    /// Only the cluster whose tab was displayed; the others connect when their tab is first shown.
    #[default]
    Active,
    /// Every restored cluster, the displayed one first, two at a time.
    All,
}

impl From<RestoreConnectSetting> for RestoreConnect {
    fn from(setting: RestoreConnectSetting) -> Self {
        match setting {
            RestoreConnectSetting::Active => Self::Active,
            RestoreConnectSetting::All => Self::All,
        }
    }
}

/// What one settings layer says about session restore: the `session` object of `settings.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct SessionRestoreContent {
    /// Reopen the clusters, tabs and namespaces of the last session at launch. Off by default.
    /// Clusters connect after the window is up, and a cluster that fails or is unreachable only
    /// shows an error in its own tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore: Option<bool>,
    /// Which restored clusters connect at launch: `active` (the one that was displayed; the others
    /// connect when you open their tab) or `all`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_connect: Option<RestoreConnectSetting>,
}

/// The resolved `session` settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionRestoreSettings {
    /// Whether launch reopens the last session.
    pub restore: bool,
    /// Which restored clusters connect.
    pub connect: RestoreConnectSetting,
}

impl Settings for SessionRestoreSettings {
    const KEY: Option<&'static str> = Some("session");
    type Content = SessionRestoreContent;

    fn from_content(content: SessionRestoreContent) -> Self {
        Self {
            restore: content.restore.unwrap_or(false),
            connect: content.restore_connect.unwrap_or_default(),
        }
    }
}

oxikube_settings::register_settings!(SessionRestoreSettings);
