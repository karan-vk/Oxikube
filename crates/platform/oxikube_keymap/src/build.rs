//! Turn the sections of one layer into GPUI [`KeyBinding`]s, validating as it goes.
//!
//! The validator and the loader are the same pass so they cannot disagree: a binding is either
//! built or reported. A bad binding never stops the others, a bad `context` skips only its
//! section, and an action nobody registered is an error in the user's file but silently
//! skipped in the embedded layers (see [`KeymapLayer::tolerates_unknown_actions`]).

use gpui::{
    App, KeyBinding, KeyBindingContextPredicate, NoAction, PlatformKeyboardMapper, SharedString,
};
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
    build_layer_with(cx, cx.keyboard_mapper().as_ref(), layer, sections)
}

/// [`build_layer`] with the platform keyboard mapper given, so a test can stand in for a
/// non-US layout (the mapper is what a section's `use_key_equivalents` flag feeds).
pub fn build_layer_with(
    cx: &App,
    mapper: &dyn PlatformKeyboardMapper,
    layer: KeymapLayer,
    sections: &[(usize, KeymapSection)],
) -> BuiltLayer {
    let mut built = BuiltLayer::default();
    for (index, section) in sections {
        let predicate = match parse_context(section) {
            Ok(predicate) => predicate,
            Err(problem) => {
                built
                    .diagnostics
                    .push(KeymapDiagnostic::section(layer, *index, problem));
                continue;
            }
        };
        for (keystrokes, value) in &section.bindings {
            let result = build_binding(
                cx,
                mapper,
                layer,
                section.use_key_equivalents,
                predicate.clone(),
                keystrokes,
                value,
            );
            match result {
                Ok(binding) => built.bindings.push(binding),
                Err(Rejected::Tolerated) => built.skipped += 1,
                Err(Rejected::Problem(problem)) => built.diagnostics.push(
                    KeymapDiagnostic::binding(layer, *index, keystrokes, problem),
                ),
            }
        }
    }
    built
}

/// Why a binding was not built.
enum Rejected {
    /// An embedded layer names an action no crate registered: skipped without a report.
    Tolerated,
    Problem(KeymapProblem),
}

impl From<KeymapProblem> for Rejected {
    fn from(problem: KeymapProblem) -> Self {
        Self::Problem(problem)
    }
}

fn parse_context(
    section: &KeymapSection,
) -> Result<Option<Rc<KeyBindingContextPredicate>>, KeymapProblem> {
    let Some(context) = section.context_expr() else {
        return Ok(None);
    };
    KeyBindingContextPredicate::parse(context)
        .map(|predicate| Some(Rc::new(predicate)))
        .map_err(|err| KeymapProblem::InvalidContext {
            context: context.to_owned(),
            message: err.to_string(),
        })
}

fn build_binding(
    cx: &App,
    mapper: &dyn PlatformKeyboardMapper,
    layer: KeymapLayer,
    use_key_equivalents: bool,
    predicate: Option<Rc<KeyBindingContextPredicate>>,
    keystrokes: &str,
    value: &Value,
) -> Result<KeyBinding, Rejected> {
    if keystrokes.split_whitespace().next().is_none() {
        return Err(KeymapProblem::InvalidKeystrokes {
            message: "a binding needs at least one keystroke".to_owned(),
        }
        .into());
    }
    let (action, input) = match KeymapAction::from_json(value)
        .map_err(|message| KeymapProblem::InvalidBinding { message })?
    {
        KeymapAction::Unbind => (Box::new(NoAction) as Box<dyn gpui::Action>, None),
        KeymapAction::Action { name, data } => {
            let input = data
                .as_ref()
                .map(|data| SharedString::from(data.to_string()));
            let action = match ActionRegistry::build(cx, &name, data) {
                Ok(action) => action,
                Err(BuildActionError::Unknown) if layer.tolerates_unknown_actions() => {
                    return Err(Rejected::Tolerated);
                }
                Err(BuildActionError::Unknown) => {
                    return Err(KeymapProblem::UnknownAction { name }.into());
                }
                Err(BuildActionError::InvalidData(message)) => {
                    return Err(KeymapProblem::InvalidActionData { name, message }.into());
                }
            };
            (action, input)
        }
    };
    KeyBinding::load(
        keystrokes,
        action,
        predicate,
        use_key_equivalents,
        input,
        mapper,
    )
    .map(|binding| binding.with_meta(layer.meta()))
    .map_err(|err| {
        KeymapProblem::InvalidKeystrokes {
            message: err.to_string(),
        }
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        DummyKeyboardMapper, KeyContext, KeybindingKeystroke, Keymap, Keystroke, TestAppContext,
        actions,
    };

    actions!(build_test, [Alpha]);

    /// A stand-in for a non-US layout (AZERTY-like): with key equivalents on, the key the
    /// binding names (`a`) is the physical key that types `q`.
    struct SwapsAForQ(DummyKeyboardMapper);

    impl PlatformKeyboardMapper for SwapsAForQ {
        fn map_key_equivalent(
            &self,
            mut keystroke: Keystroke,
            use_key_equivalents: bool,
        ) -> KeybindingKeystroke {
            if use_key_equivalents && keystroke.key == "a" {
                keystroke.key = "q".to_owned();
            }
            KeybindingKeystroke::from_keystroke(keystroke)
        }

        fn get_key_equivalents(&self) -> Option<&rustc_hash::FxHashMap<char, char>> {
            self.0.get_key_equivalents()
        }
    }

    fn sections(use_key_equivalents: bool) -> Vec<(usize, KeymapSection)> {
        let json = format!(
            r#"[{{"use_key_equivalents": {use_key_equivalents}, "bindings": {{"ctrl-a": "build_test::Alpha"}}}}]"#
        );
        crate::file::parse_keymap(&json, KeymapLayer::User)
            .unwrap()
            .sections
    }

    /// Whether typing `ctrl-<key>` runs `Alpha` under a keymap built with the flag set to `flag`.
    fn fires(cx: &mut TestAppContext, flag: bool, key: &str) -> bool {
        let built = cx.update(|cx| {
            build_layer_with(
                cx,
                &SwapsAForQ(DummyKeyboardMapper),
                KeymapLayer::User,
                &sections(flag),
            )
        });
        assert!(built.diagnostics.is_empty(), "{:?}", built.diagnostics);
        let mut keymap = Keymap::default();
        keymap.add_bindings(built.bindings);
        let typed = Keystroke::parse(&format!("ctrl-{key}")).unwrap();
        let (matches, _) = keymap.bindings_for_input(&[typed], &[KeyContext::default()]);
        matches.iter().any(|b| b.action().partial_eq(&Alpha))
    }

    #[gpui::test]
    fn use_key_equivalents_reaches_the_keyboard_mapper(cx: &mut TestAppContext) {
        // Flag on: the layout's mapping applies, so the binding answers to the remapped key.
        assert!(fires(cx, true, "q"));
        assert!(!fires(cx, true, "a"));
        // Flag off: the key is taken literally, whatever the layout.
        assert!(fires(cx, false, "a"));
        assert!(!fires(cx, false, "q"));
    }
}
