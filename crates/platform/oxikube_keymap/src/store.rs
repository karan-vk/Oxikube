//! [`KeymapStore`]: the three layers (default, vim, user), their parse state and the merge to a
//! flat binding list.
//!
//! The store never touches GPUI's keymap; [`crate::global`] does, so the merge rules are
//! testable on their own. Merging is "concatenate in layer order": GPUI resolves a conflict in
//! favour of the binding added last (and a `null` hides lower layers through the metadata of
//! [`KeymapLayer::meta`]), so the user's file wins without any per-key bookkeeping here.

use std::path::{Path, PathBuf};

use gpui::{App, KeyBinding, KeyBindingContextPredicate, Keystroke};
use oxikube_assets::KeymapPlatform;

use crate::base_keymap::BaseKeymap;
use crate::build::build_layer;
use crate::conflicts::{KeymapConflict, find_conflicts};
use crate::diagnostics::KeymapDiagnostic;
use crate::file::{KeymapAction, KeymapSection, parse_keymap};
use crate::known::KnownNames;
use crate::layer::KeymapLayer;
use crate::lines::SourceLines;
use crate::registry::ActionRegistry;
use crate::rows::LayerBinding;
use crate::validate::{sort_in_file_order, validate};

/// Sections of one layer plus the problems found reading them.
#[derive(Default)]
struct LayerState {
    sections: Vec<(usize, KeymapSection)>,
    /// Section problems, plus the whole-file error that made the layer keep older sections.
    parse_diagnostics: Vec<KeymapDiagnostic>,
    /// Where the sections are in the text, to give later problems their lines.
    lines: SourceLines,
    /// The user's file text, kept so each merge can run the validator over it with the actions
    /// registered by then (the embedded layers are ours and not validated this way).
    text: Option<String>,
    /// The sections come from an earlier text than the diagnostics: the file on disk is
    /// currently unreadable or invalid. Their binding problems were reported when that text
    /// loaded, and their lines no longer match the file, so a merge does not repeat them.
    stale: bool,
}

impl LayerState {
    fn parse(text: &str, layer: KeymapLayer) -> Self {
        match parse_keymap(text, layer) {
            Ok(parsed) => Self {
                sections: parsed.sections,
                parse_diagnostics: parsed.diagnostics,
                lines: parsed.lines,
                text: None,
                stale: false,
            },
            Err(diagnostic) => Self {
                sections: Vec::new(),
                parse_diagnostics: vec![diagnostic],
                lines: SourceLines::default(),
                text: None,
                stale: false,
            },
        }
    }
}

/// The text of a user `keymap.json`, parsed: JSON with comments to sections, with their lines.
///
/// Nothing in it touches GPUI, so it is `Send`: the file watcher builds it on its own thread and
/// hands it to the UI thread, which only has to validate the actions (they need the app's action
/// registry) and swap the keymap ([`KeymapStore::set_user_parsed`]).
pub struct ParsedUserKeymap(LayerState);

impl ParsedUserKeymap {
    /// Parse `text`, the contents of a `keymap.json` (blank for none).
    pub fn parse(text: &str) -> Self {
        let mut state = LayerState::parse(text, KeymapLayer::User);
        state.text = Some(text.to_owned());
        Self(state)
    }
}

/// How the keymap is composed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeymapOptions {
    /// Which OS's default keymap to use (the compile target unless a test says otherwise).
    pub platform: KeymapPlatform,
    /// The base keymap layered between the defaults and the user's file (`Default`: none).
    pub base_keymap: BaseKeymap,
}

impl Default for KeymapOptions {
    fn default() -> Self {
        Self {
            platform: KeymapPlatform::current(),
            base_keymap: BaseKeymap::Default,
        }
    }
}

/// The bindings of all layers merged, ready for `cx.bind_keys`.
pub struct MergedKeymap {
    /// Default, then base keymap, then user bindings, in that order.
    pub bindings: Vec<KeyBinding>,
    /// Embedded-layer bindings skipped because no crate registered their action.
    pub skipped_embedded: usize,
}

/// The layers of the keymap and their problems. A GPUI global once [`crate::init`] ran.
pub struct KeymapStore {
    options: KeymapOptions,
    default: LayerState,
    /// The parsed file of the current base keymap, when it has one.
    base: Option<LayerState>,
    user: LayerState,
    user_path: Option<PathBuf>,
    diagnostics: Vec<KeymapDiagnostic>,
    conflicts: Vec<KeymapConflict>,
}

impl KeymapStore {
    /// A store with the embedded layers parsed and no user file.
    pub fn new(options: KeymapOptions) -> Self {
        let mut store = Self {
            options: KeymapOptions {
                base_keymap: BaseKeymap::Default,
                ..options
            },
            default: LayerState::parse(
                oxikube_assets::default_keymap(options.platform),
                KeymapLayer::Default,
            ),
            base: None,
            user: LayerState::default(),
            user_path: None,
            diagnostics: Vec::new(),
            conflicts: Vec::new(),
        };
        store.set_base_keymap(options.base_keymap);
        store
    }

    /// The options the store was built with (`base_keymap` reflects later
    /// [`Self::set_base_keymap`] calls).
    pub fn options(&self) -> KeymapOptions {
        self.options
    }

    /// The base keymap that is on.
    pub fn base_keymap(&self) -> BaseKeymap {
        self.options.base_keymap
    }

    /// Whether the vim layer is on.
    pub fn vim_enabled(&self) -> bool {
        self.options.base_keymap.is_vim()
    }

    /// Switch the base keymap: drop the old overlay and parse the new one's embedded file (a few
    /// dozen lines). Returns whether the base keymap changed.
    pub fn set_base_keymap(&mut self, base: BaseKeymap) -> bool {
        let changed = self.options.base_keymap != base;
        if changed {
            self.options.base_keymap = base;
            self.base = base
                .layer()
                .zip(base.overlay_text())
                .map(|(layer, text)| LayerState::parse(text, layer));
        }
        changed
    }

    /// The parsed state of `layer`, when it is on.
    fn layer_state(&self, layer: KeymapLayer) -> Option<&LayerState> {
        match layer {
            KeymapLayer::Default => Some(&self.default),
            KeymapLayer::Vim | KeymapLayer::K9s => self
                .base
                .as_ref()
                .filter(|_| self.options.base_keymap.layer() == Some(layer)),
            KeymapLayer::User => Some(&self.user),
        }
    }

    /// Replace the user layer with the sections of `text`. Returns whether the effective
    /// sections changed.
    ///
    /// A file that is not valid keeps the previous user sections (and records why); a valid
    /// file with bad sections loads the good ones. Either way the problems are reported.
    pub fn set_user_text(&mut self, text: &str) -> bool {
        self.set_user_parsed(ParsedUserKeymap::parse(text))
    }

    /// [`Self::set_user_text`] for text that was parsed elsewhere (the watcher's thread).
    pub fn set_user_parsed(&mut self, parsed: ParsedUserKeymap) -> bool {
        let next = parsed.0;
        let whole_file_error =
            next.sections.is_empty() && next.parse_diagnostics.iter().any(|d| d.section.is_none());
        if whole_file_error {
            // Keep the sections of the last good file; only the problem is new.
            self.user.parse_diagnostics = next.parse_diagnostics;
            self.user.stale = true;
            return false;
        }
        let changed = next.sections != self.user.sections;
        self.user = next;
        changed
    }

    /// Remember where the user's file is (`None` when the keymap has no file, as in memory).
    pub fn set_user_path(&mut self, path: Option<PathBuf>) {
        self.user_path = path;
    }

    /// The user's `keymap.json`, when the keymap was installed from a config directory. The file
    /// may not exist yet.
    pub fn user_path(&self) -> Option<&Path> {
        self.user_path.as_deref()
    }

    /// The text the user layer was last loaded from: the file's text, or the last good text while
    /// the file is unreadable or invalid. `None` before any user text was loaded.
    pub fn user_text(&self) -> Option<&str> {
        self.user.text.as_deref()
    }

    /// Record that the user file exists but could not be read. The previous user sections stay
    /// in effect (none at start-up) and the problem is reported on the next merge.
    pub fn set_user_unreadable(&mut self, message: &str) {
        self.user.parse_diagnostics = vec![KeymapDiagnostic::unreadable(
            KeymapLayer::User,
            message.to_owned(),
        )];
        self.user.stale = true;
    }

    /// Merge every enabled layer into one list of GPUI bindings, in layer order, and record the
    /// problems found ([`Self::diagnostics`]).
    pub fn merge(&mut self, cx: &App) -> MergedKeymap {
        let mut bindings = Vec::new();
        let mut diagnostics = Vec::new();
        let mut conflicts = Vec::new();
        let mut skipped_embedded = 0;
        for layer in KeymapLayer::ORDER {
            let Some(state) = self.layer_state(layer) else {
                continue;
            };
            let built = build_layer(cx, layer, &state.sections);
            // Only the user's layer keeps its text: the one validator reports everything it can
            // see in it, unless the sections are stale.
            let validated =
                state.text.as_deref().filter(|_| !state.stale).map(|text| {
                    validate(text, &ActionRegistry::from_app(cx), &KnownNames::current())
                });
            if let Some(mut found) = validated {
                // What only building can tell (an action rejecting its data) is added.
                for mut extra in built.diagnostics {
                    extra.locate(&state.lines);
                    let seen = found.iter().any(|d| {
                        d.section == extra.section
                            && d.keystrokes == extra.keystrokes
                            && d.code() == extra.code()
                    });
                    if !seen {
                        found.push(extra);
                    }
                }
                sort_in_file_order(&mut found);
                diagnostics.extend(found);
                conflicts.extend(find_conflicts(layer, &state.sections, &state.lines));
            } else if state.stale {
                // Sections of an earlier file: their problems and line numbers are history.
                diagnostics.extend(state.parse_diagnostics.iter().cloned());
                conflicts.extend(find_conflicts(
                    layer,
                    &state.sections,
                    &SourceLines::default(),
                ));
            } else {
                diagnostics.extend(state.parse_diagnostics.iter().cloned());
                diagnostics.extend(built.diagnostics.into_iter().map(|mut diagnostic| {
                    diagnostic.locate(&state.lines);
                    diagnostic
                }));
                conflicts.extend(find_conflicts(layer, &state.sections, &state.lines));
            }
            skipped_embedded += built.skipped;
            bindings.extend(built.bindings);
        }
        self.diagnostics = diagnostics;
        self.conflicts = conflicts;
        MergedKeymap {
            bindings,
            skipped_embedded,
        }
    }

    /// Every binding of every enabled layer, weakest first (default, base keymap, user; sections and
    /// bindings in file order), as the files state them, each context in the predicate's own
    /// spelling (`A&&B` is `A && B`). What the loader rejects is left out like the loader leaves
    /// it out (its problem is in [`Self::diagnostics`]): a section whose context does not parse,
    /// a binding whose keystrokes or value do not. Whether the action exists is the caller's
    /// check. The keymap editor builds its rows from this ([`crate::rows::build_rows`]).
    pub fn layer_bindings(&self) -> Vec<LayerBinding> {
        let mut out = Vec::new();
        for layer in KeymapLayer::ORDER {
            let Some(state) = self.layer_state(layer) else {
                continue;
            };
            for (_, section) in &state.sections {
                let context = match section.context_expr() {
                    None => None,
                    Some(text) => match KeyBindingContextPredicate::parse(text) {
                        Ok(predicate) => Some(predicate.to_string()),
                        Err(_) => continue,
                    },
                };
                for (keystrokes, value) in &section.bindings {
                    if !keystrokes_parse(keystrokes) {
                        continue;
                    }
                    if let Ok(action) = KeymapAction::from_json(value) {
                        out.push(LayerBinding {
                            layer,
                            context: context.clone(),
                            keystrokes: keystrokes.clone(),
                            action,
                        });
                    }
                }
            }
        }
        out
    }

    /// The problems found by the last merge, for the UI to surface.
    pub fn diagnostics(&self) -> &[KeymapDiagnostic] {
        &self.diagnostics
    }

    /// The problems of the user's file alone, which is what the notification reports: the
    /// embedded layers' are bugs of ours, logged, not something for the user to fix.
    pub fn user_diagnostics(&self) -> Vec<KeymapDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.layer == KeymapLayer::User)
            .cloned()
            .collect()
    }

    /// Keys given two meanings in one context of one layer by the last merge.
    pub fn conflicts(&self) -> &[KeymapConflict] {
        &self.conflicts
    }
}

/// Whether `keystrokes` is one or more keystrokes GPUI can read, as the loader requires.
fn keystrokes_parse(keystrokes: &str) -> bool {
    let mut parts = keystrokes.split_whitespace().peekable();
    parts.peek().is_some() && parts.all(|part| Keystroke::parse(part).is_ok())
}
