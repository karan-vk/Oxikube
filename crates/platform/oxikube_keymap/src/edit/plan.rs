//! From the commands' payloads (`keymap::SetBinding`, `keymap::RemoveBinding`) to the
//! [`KeybindUpdateOperation`]s that write them, and back from the editor's rows to payloads.

use oxikube_domain::command::{KeymapBindingEdit, KeymapBindingRef, KeymapSource};
use serde_json::Value;

use super::ops::{BindingLocation, KeybindTarget, KeybindUpdateOperation, normalise_context};
use crate::conflicts::normalise;
use crate::file::KeymapAction;
use crate::layer::KeymapLayer;
use crate::rows::KeymapRow;

/// Why a payload does not describe an edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanError(pub String);

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PlanError {}

/// The layer of a payload's source.
pub fn layer_of(source: KeymapSource) -> KeymapLayer {
    match source {
        KeymapSource::Default => KeymapLayer::Default,
        KeymapSource::Base => KeymapLayer::Vim,
        KeymapSource::User => KeymapLayer::User,
    }
}

/// The payload source of a layer.
pub fn source_of(layer: KeymapLayer) -> KeymapSource {
    match layer {
        KeymapLayer::Default => KeymapSource::Default,
        KeymapLayer::Vim => KeymapSource::Base,
        KeymapLayer::User => KeymapSource::User,
    }
}

/// Where `binding` is.
pub fn location_of(binding: &KeymapBindingRef) -> BindingLocation {
    BindingLocation {
        context: binding.context.clone(),
        keystrokes: binding.keystrokes.clone(),
        layer: layer_of(binding.source),
    }
}

/// The reference to a bound row of the keymap editor (`None` for a command bound nowhere).
pub fn ref_of_row(row: &KeymapRow) -> Option<KeymapBindingRef> {
    let layer = row.source?;
    if row.keystrokes.is_empty() {
        return None;
    }
    Some(KeymapBindingRef {
        context: row.context.clone(),
        keystrokes: row.keystrokes_text(),
        source: source_of(layer),
    })
}

/// The operations of `keymap::SetBinding`: the binding (in place of what it replaces), then the
/// unbinding of each conflict the user chose to replace. An unbind the new binding already
/// covers (the same keys in the same context) is dropped: the user binding outranks a default
/// there, and a user binding there is overwritten by it.
///
/// # Errors
///
/// The data is not JSON, or the keystrokes are blank.
pub fn set_binding_operations(
    edit: &KeymapBindingEdit,
) -> Result<Vec<KeybindUpdateOperation>, PlanError> {
    if edit.keystrokes.split_whitespace().next().is_none() {
        return Err(PlanError(
            "a binding needs at least one keystroke".to_owned(),
        ));
    }
    let data = match edit.data.as_deref() {
        None => None,
        Some(text) => Some(
            serde_json::from_str::<Value>(text)
                .map_err(|err| PlanError(format!("the action's data is not JSON: {err}")))?,
        ),
    };
    let with = KeybindTarget {
        context: edit
            .context
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_owned),
        keystrokes: edit
            .keystrokes
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        action: KeymapAction::Action {
            name: edit.action.clone(),
            data,
        },
    };
    let place = (
        normalise_context(with.context.as_deref()),
        normalise(&with.keystrokes),
    );
    let mut operations = vec![match &edit.replaces {
        Some(target) => KeybindUpdateOperation::Replace {
            target: location_of(target),
            with,
        },
        None => KeybindUpdateOperation::Add(with),
    }];
    for other in &edit.unbind {
        let at = (
            normalise_context(other.context.as_deref()),
            normalise(&other.keystrokes),
        );
        if at == place || edit.replaces.as_ref() == Some(other) {
            continue;
        }
        operations.push(KeybindUpdateOperation::Remove {
            target: location_of(other),
        });
    }
    Ok(operations)
}

/// The operation of `keymap::RemoveBinding`.
pub fn remove_binding_operations(binding: &KeymapBindingRef) -> Vec<KeybindUpdateOperation> {
    vec![KeybindUpdateOperation::Remove {
        target: location_of(binding),
    }]
}
