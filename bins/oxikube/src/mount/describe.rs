//! The `describe` setting into the describe adapter (E07-S06): which backend renders the Describe
//! tab, and where `kubectl` is.
//!
//! The adapter (`oxikube_describe`) reads a shared [`DescribePreference`] at each describe, so a
//! change of `describe.backend` or `describe.kubectl_path` in `settings.json` applies to the next
//! describe of every open connection, without reconnecting.

use gpui::App;
use oxikube_describe::{Backend, DescribeConfig, DescribePreference};
use oxikube_resources_ui::{DescribeBackendSetting, DescribeSettings};
use oxikube_settings::Settings as _;

/// Sets `preference` from the current settings and again on every change of them. Without a
/// settings store it keeps the defaults (`auto`, `kubectl` on `PATH`).
pub fn follow_settings(preference: &DescribePreference, cx: &mut App) {
    apply(preference, cx);
    let preference = preference.clone();
    DescribeSettings::observe(cx, move |cx| apply(&preference, cx)).detach();
}

/// The adapter's configuration for `settings`.
pub fn config_for(settings: &DescribeSettings) -> DescribeConfig {
    DescribeConfig {
        backend: match settings.backend {
            DescribeBackendSetting::Auto => Backend::Auto,
            DescribeBackendSetting::Native => Backend::Native,
            DescribeBackendSetting::Kubectl => Backend::Kubectl,
        },
        kubectl_path: settings.kubectl_path.as_deref().map(Into::into),
    }
}

fn apply(preference: &DescribePreference, cx: &App) {
    if let Some(settings) = DescribeSettings::try_get(cx) {
        preference.set(config_for(settings));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_map_onto_the_adapters_configuration() {
        let config = config_for(&DescribeSettings {
            backend: DescribeBackendSetting::Kubectl,
            kubectl_path: Some("/opt/bin/kubectl".into()),
        });
        assert_eq!(config.backend, Backend::Kubectl);
        assert_eq!(config.kubectl_path, Some("/opt/bin/kubectl".into()));
        assert_eq!(
            config_for(&DescribeSettings::default()),
            DescribeConfig::default()
        );
    }
}
