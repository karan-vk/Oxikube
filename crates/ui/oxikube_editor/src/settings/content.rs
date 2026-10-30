//! [`EditorSettingsContent`]: what one layer of `settings.json` may say in its `editor` object.
//! Free of gpui types, so the settings page (E21) can bind to its field paths.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What one settings layer says about the manifest editor: the `editor` object of `settings.json`
/// (and of `clusters.<id>`, where it overrides the layers above for that cluster's editors).
///
/// Every field is optional: a layer sets any subset. The shipped `default.json` spells out all of
/// them. A value out of range or of the wrong type is rejected with a notice and the default is
/// used for that field alone; the rest of the block still applies.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EditorSettingsContent {
    /// Spaces one indent is (the tab key, indent and outdent): 1 to 16. Kubernetes manifests
    /// conventionally use 2. A change applies to open editors at once and keeps their text and
    /// undo history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 16))]
    pub tab_size: Option<i64>,
    /// The font family of the text, such as "JetBrains Mono". `null` uses the theme's monospace
    /// font. A change applies to open editors at once.
    #[schemars(extend("x-oxikube-dynamic-enum" = "fonts"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// The font size in points (6 to 72), at 100 % UI zoom: the UI zoom scales it like every
    /// other size, so 14 is 21 pixels at a zoom of 1.5. `null` uses the theme's monospace size.
    /// A change applies to open editors at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 6, max = 72))]
    pub font_size: Option<f64>,
    /// Wrap long lines at the editor's width instead of scrolling sideways. A change applies to
    /// open editors at once (the editor's Wrap toggle changes one editor only, until the next
    /// change of this setting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_wrap: Option<bool>,
    /// Show `metadata.managedFields` when a live object is opened. They are long and rarely
    /// useful, so they are hidden (stripped from the text) unless this is `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_managed_fields: Option<bool>,
    /// Show the values of a Secret's `data` as `********` when the object is opened. Turning it
    /// off shows the values in the editor, in memory only: Oxikube never writes decoded Secret
    /// data to disk, logs or the audit trail, whatever this says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_secrets: Option<bool>,
    /// Check the manifest against the cluster's schemas as you type (squiggles, the problem
    /// count). Turn it off for a very large manifest or a cluster whose schemas are slow. Turning
    /// it on checks every open editor at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<bool>,
    /// The field manager name server-side apply records for the fields this editor sets
    /// (`kubectl apply --server-side --field-manager`). 1 to 128 characters, no whitespace; an
    /// invalid name is replaced by `oxikube` with a notice. A change applies to the next apply.
    /// Set it under `clusters.<id>.editor` to use another name on one cluster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 128), regex(pattern = r"^\S+$"))]
    pub field_manager: Option<String>,
}
