//! The layer badge of a field: which settings layer its value comes from, and the words of its
//! tooltip. Plain data; the view picks the colours.

use oxikube_settings::SettingSource;

/// The order the layers apply in, for every tooltip.
const PRECEDENCE: &str =
    "Layers apply in order: default.json, then your settings.json, then the cluster's block.";

/// What the badge of a field says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerBadge {
    /// The layer the value comes from.
    pub source: SettingSource,
}

impl LayerBadge {
    /// The badge of a value from `source`.
    pub fn new(source: SettingSource) -> Self {
        Self { source }
    }

    /// The badge's text: `Default`, `User`, `Cluster`.
    pub fn label(self) -> &'static str {
        self.source.label()
    }

    /// The tooltip: where the value comes from, and how the layers stack.
    pub fn tooltip(self) -> String {
        let origin = match self.source {
            SettingSource::Default => "Built-in default: not set in your settings.json.",
            SettingSource::User => "Set in your settings.json.",
            SettingSource::Cluster => "Set for this cluster in settings.json (clusters.<id>).",
        };
        format!("{origin} {PRECEDENCE}")
    }

    /// Whether the value differs from the built-in default's layer, so the badge stands out.
    pub fn is_override(self) -> bool {
        self.source != SettingSource::Default
    }
}
