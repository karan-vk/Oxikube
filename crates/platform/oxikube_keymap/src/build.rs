//! Turn the sections of one layer into GPUI [`KeyBinding`]s, validating as it goes.
//!
//! The validator and the loader are the same pass so they cannot disagree: a binding is either
//! built or reported. A bad binding never stops the others, a bad `context` skips only its
//! section, and an action nobody registered is an error in the user's file but silently
//! skipped in the embedded layers (see [`KeymapLayer::tolerates_unknown_actions`]).

use gpui::{App, KeyBinding, KeyBindingContextPredicate, NoAction, SharedString};
use serde_json::Value;
use std::rc::Rc;

use crate::diagnostics::{KeymapDiagnostic, KeymapProblem};
use crate::file::{KeymapAction, KeymapSection};
use crate::layer::KeymapLayer;
use crate::registry::{ActionRegistry, BuildActionError};

/// What building a layer produced.
#[derive(Default)]
pub struct BuiltLayer {
    /// The bindings, in the order GPUI should add them.
    pub bindings: Vec<KeyBinding>,
    /// Everything that was rejected.
    pub diagnostics: Vec<KeymapDiagnostic>,
    /// Bindings of an embedded layer skipped because their action is not registered.
    pub skipped: usize,
}

/// Build every binding of `sections` (each with its index in the file) for `layer`.
pub fn build_layer(
    cx: &App,
    layer: KeymapLayer,
    sections: &[(usize, KeymapSection)],
) -> BuiltLayer {
    let mut built = BuiltLayer::default();
    let mapper = cx.keyboard_mapper().clone();
    for (index, section) in sections {
        let predicate = match section.context_expr() {
            None => None,
            Some(context) => match KeyBindingContextPredicate::parse(context) {
                Ok(predicate) => Some(Rc::new(predicate)),
                Err(err) => {
                    built.diagnostics.push(KeymapDiagnostic::section(
                        layer,
                        *index,
                        KeymapProblem::InvalidContext {
                            context: context.to_owned(),
                            message: err.to_string(),
                        },
                    ));
                    continue;
                }
            },
        };
        for (keystrokes, value) in &section.bindings {
            if keystrokes.split_whitespace().next().is_none() {
                built.diagnostics.push(KeymapDiagnostic::binding(
                    layer,
                    *index,
                    keystrokes,
                    KeymapProblem::InvalidKeystrokes {
                        message: "a binding needs at least one keystroke".to_owned(),
                    },
                ));
                continue;
            }
            let action = match KeymapAction::from_json(value) {
                Ok(action) => action,
                Err(message) => {
                    built.diagnostics.push(KeymapDiagnostic::binding(
                        layer,
                        *index,
                        keystrokes,
                        KeymapProblem::InvalidBinding { message },
                    ));
                    continue;
                }
            };
            let (action, input) = match action {
                KeymapAction::Unbind => (Box::new(NoAction) as Box<dyn gpui::Action>, None),
                KeymapAction::Action { name, data } => {
                    let input = data.as_ref().map(Value::to_string).map(SharedString::from);
                    match ActionRegistry::build(cx, &name, data) {
                        Ok(action) => (action, input),
                        Err(BuildActionError::Unknown) if layer.tolerates_unknown_actions() => {
                            built.skipped += 1;
                            continue;
                        }
                        Err(err) => {
                            built.diagnostics.push(KeymapDiagnostic::binding(
                                layer,
                                *index,
                                keystrokes,
                                match err {
                                    BuildActionError::Unknown => {
                                        KeymapProblem::UnknownAction { name }
                                    }
                                    BuildActionError::InvalidData(message) => {
                                        KeymapProblem::InvalidActionData { name, message }
                                    }
                                },
                            ));
                            continue;
                        }
                    }
                }
            };
            match KeyBinding::load(
                keystrokes,
                action,
                predicate.clone(),
                section.use_key_equivalents,
                input,
                mapper.as_ref(),
            ) {
                Ok(binding) => built.bindings.push(binding.with_meta(layer.meta())),
                Err(err) => built.diagnostics.push(KeymapDiagnostic::binding(
                    layer,
                    *index,
                    keystrokes,
                    KeymapProblem::InvalidKeystrokes {
                        message: err.to_string(),
                    },
                )),
            }
        }
    }
    built
}
