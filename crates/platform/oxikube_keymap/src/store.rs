//! [`KeymapStore`]: the three layers (default, vim, user), their parse state and the merge to a
//! flat binding list.
//!
//! The store never touches GPUI's keymap; [`crate::global`] does, so the merge rules are
//! testable on their own. Merging is "concatenate in layer order": GPUI resolves a conflict in
//! favour of the binding added last (and a `null` hides lower layers through the metadata of
//! [`KeymapLayer::meta`]), so the user's file wins without any per-key bookkeeping here.

use gpui::{App, KeyBinding};
use oxikube_assets::KeymapPlatform;

use crate::build::build_layer;
use crate::diagnostics::KeymapDiagnostic;
use crate::file::{KeymapSection, parse_keymap};
use crate::layer::KeymapLayer;

/// Sections of one layer plus the problems found reading them.
#[derive(Default)]
struct LayerState {
    sections: Vec<(usize, KeymapSection)>,
    /// Section problems, plus the whole-file error that made the layer keep older sections.
    parse_diagnostics: Vec<KeymapDiagnostic>,
}

impl LayerState {
    fn parse(text: &str, layer: KeymapLayer) -> Self {
        match parse_keymap(text, layer) {
            Ok(parsed) => Self {
                sections: parsed.sections,
                parse_diagnostics: parsed.diagnostics,
            },
            Err(diagnostic) => Self {
                sections: Vec::new(),
                parse_diagnostics: vec![diagnostic],
            },
        }
    }
}

/// How the keymap is composed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeymapOptions {
    /// Which OS's default keymap to use (the compile target unless a test says otherwise).
    pub platform: KeymapPlatform,
    /// Layer the optional `vim.json` between the defaults and the user's file.
    pub vim: bool,
}

impl Default for KeymapOptions {
    fn default() -> Self {
        Self {
            platform: KeymapPlatform::current(),
            vim: false,
        }
    }
}

/// The bindings of all layers merged, ready for `cx.bind_keys`.
pub struct MergedKeymap {
    /// Default, then vim, then user bindings, in that order.
    pub bindings: Vec<KeyBinding>,
    /// Embedded-layer bindings skipped because no crate registered their action.
    pub skipped_embedded: usize,
}

/// The layers of the keymap and their problems. A GPUI global once [`crate::init`] ran.
pub struct KeymapStore {
    options: KeymapOptions,
    default: LayerState,
    vim: Option<LayerState>,
    user: LayerState,
    diagnostics: Vec<KeymapDiagnostic>,
}

impl KeymapStore {
    /// A store with the embedded layers parsed and no user file.
    pub fn new(options: KeymapOptions) -> Self {
        let mut store = Self {
            options,
            default: LayerState::parse(
                oxikube_assets::default_keymap(options.platform),
                KeymapLayer::Default,
            ),
            vim: None,
            user: LayerState::default(),
            diagnostics: Vec::new(),
        };
        store.set_vim(options.vim);
        store
    }

    /// The options the store was built with (the vim flag reflects later [`Self::set_vim`]).
    pub fn options(&self) -> KeymapOptions {
        self.options
    }

    /// Whether the vim layer is on.
    pub fn vim_enabled(&self) -> bool {
        self.options.vim
    }

    /// Turn the vim layer on or off. The embedded file is parsed the first time it is needed.
    /// Returns whether the flag changed.
    pub fn set_vim(&mut self, enabled: bool) -> bool {
        let changed = self.options.vim != enabled;
        self.options.vim = enabled;
        if enabled && self.vim.is_none() {
            self.vim = Some(LayerState::parse(
                oxikube_assets::vim_keymap(),
                KeymapLayer::Vim,
            ));
        }
        changed
    }

    /// Replace the user layer with the sections of `text`. Returns whether the effective
    /// sections changed.
    ///
    /// A file that is not valid keeps the previous user sections (and records why); a valid
    /// file with bad sections loads the good ones. Either way the problems are reported.
    pub fn set_user_text(&mut self, text: &str) -> bool {
        let next = LayerState::parse(text, KeymapLayer::User);
        let whole_file_error =
            next.sections.is_empty() && next.parse_diagnostics.iter().any(|d| d.section.is_none());
        if whole_file_error {
            // Keep the sections of the last good file; only the problem is new.
            self.user.parse_diagnostics = next.parse_diagnostics;
            return false;
        }
        let changed = next.sections != self.user.sections;
        self.user = next;
        changed
    }

    /// Merge every enabled layer into one list of GPUI bindings, in layer order, and record the
    /// problems found ([`Self::diagnostics`]).
    pub fn merge(&mut self, cx: &App) -> MergedKeymap {
        let mut bindings = Vec::new();
        let mut diagnostics = Vec::new();
        let mut skipped_embedded = 0;
        for layer in KeymapLayer::ORDER {
            let state = match layer {
                KeymapLayer::Default => Some(&self.default),
                KeymapLayer::Vim => self.vim.as_ref().filter(|_| self.options.vim),
                KeymapLayer::User => Some(&self.user),
            };
            let Some(state) = state else { continue };
            diagnostics.extend(state.parse_diagnostics.iter().cloned());
            let built = build_layer(cx, layer, &state.sections);
            diagnostics.extend(built.diagnostics);
            skipped_embedded += built.skipped;
            bindings.extend(built.bindings);
        }
        self.diagnostics = diagnostics;
        MergedKeymap {
            bindings,
            skipped_embedded,
        }
    }

    /// The problems found by the last merge, for the UI to surface.
    pub fn diagnostics(&self) -> &[KeymapDiagnostic] {
        &self.diagnostics
    }
}
