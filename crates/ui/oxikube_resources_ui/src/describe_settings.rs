//! The `describe` setting (E07-S06): which implementation renders the Describe tab.
//!
//! ```json
//! "describe": { "backend": "auto", "kubectl_path": null }
//! ```
//!
//! `auto` renders in process (deskribe: 36 specialised kinds and a generic layout for custom
//! resources) and falls back to the `kubectl` binary for a kind it does not cover; `native` never
//! runs `kubectl`; `kubectl` always does. `kubectl_path` names the binary (default: `kubectl` on
//! `PATH`). A change applies to the next describe, without reconnecting: the binary follows the
//! setting ([`DescribeSettings::observe`](oxikube_settings::Settings::observe)) and hands it to
//! the describe adapter.

use oxikube_settings::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which implementation describes an object (`describe.backend`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DescribeBackendSetting {
    /// deskribe for every kind it covers, `kubectl describe` for the rest (when installed).
    #[default]
    Auto,
    /// deskribe only: a kind it does not cover is reported as unsupported.
    Native,
    /// `kubectl describe` only.
    Kubectl,
}

/// What one settings layer says about describe: the `describe` object of `settings.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct DescribeSettingsContent {
    /// `auto` (deskribe, then `kubectl` for a kind it does not cover), `native` or `kubectl`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<DescribeBackendSetting>,
    /// Path of the `kubectl` binary; unset looks `kubectl` up on `PATH`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kubectl_path: Option<String>,
}

/// The resolved `describe` settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DescribeSettings {
    /// Which implementation to use.
    pub backend: DescribeBackendSetting,
    /// The `kubectl` binary, when it is not on `PATH`.
    pub kubectl_path: Option<String>,
}

impl Settings for DescribeSettings {
    const KEY: Option<&'static str> = Some("describe");
    type Content = DescribeSettingsContent;

    fn from_content(content: DescribeSettingsContent) -> Self {
        Self {
            backend: content.backend.unwrap_or_default(),
            kubectl_path: content.kubectl_path.filter(|path| !path.trim().is_empty()),
        }
    }
}

oxikube_settings::register_settings!(DescribeSettings);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_layer_is_auto_with_kubectl_on_the_path() {
        let settings = DescribeSettings::from_content(DescribeSettingsContent::default());
        assert_eq!(settings.backend, DescribeBackendSetting::Auto);
        assert_eq!(settings.kubectl_path, None);
    }

    #[test]
    fn the_backend_reads_from_json_and_a_blank_path_is_unset() {
        let content: DescribeSettingsContent =
            serde_json::from_str(r#"{"backend": "kubectl", "kubectl_path": "  "}"#).unwrap();
        let settings = DescribeSettings::from_content(content);
        assert_eq!(settings.backend, DescribeBackendSetting::Kubectl);
        assert_eq!(settings.kubectl_path, None);
        let content: DescribeSettingsContent =
            serde_json::from_str(r#"{"backend": "native", "kubectl_path": "/opt/bin/kubectl"}"#)
                .unwrap();
        let settings = DescribeSettings::from_content(content);
        assert_eq!(settings.backend, DescribeBackendSetting::Native);
        assert_eq!(settings.kubectl_path.as_deref(), Some("/opt/bin/kubectl"));
    }
}
