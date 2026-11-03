//! The precedence view as plain data: what each layer says about a field, which one wins, the
//! explanation of the order, and the warning for an override that weakens a protection.

use oxikube_settings::{LayerValues, SettingSource};
use serde_json::Value;

/// The panel text on how the layers stack.
pub const EXPLANATION: &str = "Layers apply in order: default.json, then your settings.json, then the cluster's block (clusters.<id>). The last layer that sets a field wins; the ones before it show through for every field it leaves alone.";

/// What the tab's edits do, shown under [`EXPLANATION`].
pub const EDITS_ONLY_THE_CLUSTER: &str = "Edits here change only this cluster's block of settings.json. Your global values and other clusters are not touched; \"Reset for this cluster\" removes the key again.";

/// A value is cut to this many characters in the strip.
const VALUE_CHARS: usize = 28;

/// One layer in the strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrecedenceCell {
    /// The layer.
    pub layer: SettingSource,
    /// Its value as compact JSON (cut), or `None` when the layer does not set the field.
    pub value: Option<String>,
    /// Whether this layer's value is the one in effect.
    pub winner: bool,
}

/// The three layers of one field, lowest first, with the winner marked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Precedence {
    /// Default, user, cluster.
    pub cells: [PrecedenceCell; 3],
    /// The layer in effect.
    pub winner: SettingSource,
}

impl Precedence {
    /// The strip for `values`, with `winner` (`SettingsStore::field_source`) in effect.
    pub fn new(values: &LayerValues, winner: SettingSource) -> Self {
        let cell = |layer, value: &Option<Value>| PrecedenceCell {
            layer,
            value: value.as_ref().map(compact),
            winner: layer == winner,
        };
        Self {
            cells: [
                cell(SettingSource::Default, &values.default),
                cell(SettingSource::User, &values.user),
                cell(SettingSource::Cluster, &values.cluster),
            ],
            winner,
        }
    }

    /// Whether the cluster overrides the field.
    pub fn overridden(&self) -> bool {
        self.winner == SettingSource::Cluster
    }

    /// The per-field sentence: where the value in effect comes from.
    pub fn summary(&self) -> &'static str {
        match self.winner {
            SettingSource::Default => "Built-in default: nothing overrides it.",
            SettingSource::User => "Set in your settings.json for every cluster.",
            SettingSource::Cluster => "Overridden for this cluster.",
        }
    }
}

fn compact(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > VALUE_CHARS {
        let cut: String = text.chars().take(VALUE_CHARS).collect();
        format!("{cut}\u{2026}")
    } else {
        text
    }
}

/// A warning when the cluster's own value at `path` weakens a protection that `inherited` (the
/// value without the cluster layer) gives, else `None`. Never blocks the write.
///
/// * `read_only` off for a cluster while it is on for the others;
/// * `exec_in_read_only` on for a cluster while it is off for the others;
/// * `guard.default_read_only` off for a cluster while it is on for the others.
pub fn protection_warning(
    path: &str,
    inherited: Option<&Value>,
    own: Option<&Value>,
) -> Option<&'static str> {
    let on = |value: Option<&Value>| value == Some(&Value::Bool(true));
    let off = |value: Option<&Value>| value == Some(&Value::Bool(false));
    match path {
        "read_only" if off(own) && on(inherited) => {
            Some("Turns read-only off for this cluster while it is on for the others.")
        }
        "exec_in_read_only" if on(own) && !on(inherited) => {
            Some("Allows shells into pods of this cluster while it is read-only.")
        }
        "guard.default_read_only" if off(own) && on(inherited) => {
            Some("Stops new connections to this cluster from starting read-only.")
        }
        _ => None,
    }
}
