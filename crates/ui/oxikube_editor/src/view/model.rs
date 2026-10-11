//! [`ManifestModel`]: what the manifest editor decides, written against [`EditorApi`] only, so
//! it is tested without a window (`tests` below use a plain struct as the editor).
//!
//! It keeps the schemas the buffer's kinds need (which to fetch, which arrived, which the cluster
//! does not have), drops a validation result computed for an older buffer version, counts the
//! problems the toolbar shows and how far a large buffer's pass has got, caches the latest parse
//! (a schema arriving re-validates without re-parsing) and counts the passes ([`PassStats`]).

use std::collections::HashSet;
use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ui::editor::{DiagnosticLevel, EditorApi};

use super::progressive::Coverage;
use super::validation::{KnownSchemas, Validation};
use crate::json::{Language, LanguageMode};
use crate::yaml::{ParseCache, ParseResult};

/// How many problems the shown diagnostics hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Problems {
    /// Errors: the API server will reject the manifest.
    pub errors: usize,
    /// Warnings: probably a mistake.
    pub warnings: usize,
}

/// What [`ManifestModel::accept`] did with a validation result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accepted {
    /// The result was for the buffer as it is: its diagnostics are shown.
    Shown,
    /// The buffer changed since, or the user switched the language: the result was dropped.
    Stale,
}

/// What the validation passes of one editor did, for tests and the perf scenario.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassStats {
    /// Passes started: the debounce elapsed (or a schema arrived) and the buffer was taken.
    pub started: u64,
    /// Passes whose every chunk was checked for the buffer as it still was.
    pub completed: u64,
    /// Passes stopped because the buffer changed (or a newer pass replaced them).
    pub cancelled: u64,
    /// Parses run (a pass over a version already parsed reuses the cached parse).
    pub parses: u64,
    /// Chunks checked, by all passes.
    pub chunks: u64,
}

/// The manifest editor's state beyond the buffer: the schemas the buffer's kinds need (which to
/// fetch, which arrived, which the cluster does not have), the buffer version whose diagnostics
/// are shown (a result computed for an older version is dropped) and the problem counts the
/// toolbar shows. Written against [`EditorApi`] only, so it is tested without a window.
#[derive(Debug, Default)]
pub struct ManifestModel {
    known: KnownSchemas,
    fetching: HashSet<Gvk>,
    unavailable: Vec<(Gvk, String)>,
    problems: Problems,
    shown: Option<u64>,
    mode: LanguageMode,
    language: Language,
    coverage: Coverage,
    parsed: ParseCache,
    stats: PassStats,
}

impl ManifestModel {
    /// A model that knows no schema yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The schemas known so far, to validate against (cloned into the background task).
    pub fn known(&self) -> &KnownSchemas {
        &self.known
    }

    /// The problems of the diagnostics shown now.
    pub fn problems(&self) -> Problems {
        self.problems
    }

    /// How the language is chosen: from the content, or fixed.
    pub fn mode(&self) -> LanguageMode {
        self.mode
    }

    /// The language the buffer is read as now (highlighting, checks).
    pub fn language(&self) -> Language {
        self.language
    }

    /// Fixes the language, or lets the content decide again ([`LanguageMode::Auto`], which keeps
    /// the language shown until the next check has looked at the content). Results computed
    /// under another mode are dropped. The editor's text is untouched.
    pub fn set_mode(&mut self, mode: LanguageMode) {
        self.mode = mode;
        if let LanguageMode::Fixed(language) = mode {
            self.language = language;
        }
        self.problems = Problems::default();
        self.shown = None;
        self.coverage = Coverage::default();
    }

    /// Records the language the editor was created with (before any check has run).
    pub(super) fn adopt(&mut self, language: Language) {
        self.language = language;
    }

    /// The buffer version whose diagnostics are shown, if any.
    pub fn shown_version(&self) -> Option<u64> {
        self.shown
    }

    /// How much of the buffer the shown diagnostics cover (a large buffer fills in chunk by chunk).
    pub fn coverage(&self) -> Coverage {
        self.coverage
    }

    /// What the validation passes did so far.
    pub fn stats(&self) -> PassStats {
        self.stats
    }

    /// Counts for the passes (the view's pipeline updates them).
    pub(super) fn stats_mut(&mut self) -> &mut PassStats {
        &mut self.stats
    }

    /// The cached parse of `version`, if the last parse was of it.
    pub fn parsed(&self, version: u64) -> Option<Arc<ParseResult>> {
        self.parsed.get(version)
    }

    /// Keeps `parsed` as the parse of `version`; the previous one is released.
    pub fn cache_parse(&mut self, version: u64, parsed: Arc<ParseResult>) {
        self.parsed.insert(version, parsed);
    }

    /// Kinds whose schema could not be fetched (not "the cluster has none"), with the reason.
    pub fn unavailable(&self) -> &[(Gvk, String)] {
        &self.unavailable
    }

    /// The buffer changed: the shown diagnostics are gone (the editor drops them on an edit).
    pub fn changed(&mut self) {
        self.problems = Problems::default();
        self.shown = None;
        self.coverage = Coverage::default();
    }

    /// Shows `result` when it is for the buffer's current version and language mode; drops it
    /// otherwise. Adopts the language the content showed under [`LanguageMode::Auto`] and tells
    /// the editor to highlight with it.
    pub fn accept(&mut self, editor: &mut dyn EditorApi, result: Validation) -> Accepted {
        if result.version != editor.version() || result.mode != self.mode {
            return Accepted::Stale;
        }
        self.language = result.language;
        if editor.language() != self.language.grammar() {
            editor.set_language(self.language.grammar());
        }
        let count = |level| {
            result
                .diagnostics
                .iter()
                .filter(|d| d.level == level)
                .count()
        };
        self.problems = Problems {
            errors: count(DiagnosticLevel::Error),
            warnings: count(DiagnosticLevel::Warning),
        };
        self.shown = Some(result.version);
        self.coverage = result.coverage;
        editor.set_diagnostics(result.diagnostics);
        Accepted::Shown
    }

    /// Validation is off: drops the diagnostics shown and the problem counts.
    pub fn clear(&mut self, editor: &mut dyn EditorApi) {
        // After an edit nothing is shown (the editor dropped it): nothing to do per keystroke.
        self.coverage = Coverage::default();
        if self.shown.take().is_some() {
            self.problems = Problems::default();
            editor.set_diagnostics(Vec::new());
        }
    }

    /// Which of `missing` to fetch now: those neither known nor already being fetched. They are
    /// marked as being fetched.
    pub fn to_fetch(&mut self, missing: &[Gvk]) -> Vec<Gvk> {
        missing
            .iter()
            .filter(|gvk| !self.known.contains_key(*gvk) && self.fetching.insert((*gvk).clone()))
            .cloned()
            .collect()
    }

    /// Gives up fetching `gvk` for now (the cluster is not connected): the next validation that
    /// needs it asks again.
    pub fn forget_fetch(&mut self, gvk: &Gvk) {
        self.fetching.remove(gvk);
    }

    /// Records the answer for `gvk`'s schema. A kind the cluster does not serve, or one that
    /// failed, is known as "no schema" so it is not fetched again while this editor is open; a
    /// failure other than "not found" is kept to show.
    pub fn schema_arrived(&mut self, gvk: Gvk, outcome: OxiResult<Arc<JsonSchema>>) {
        self.fetching.remove(&gvk);
        match outcome {
            Ok(schema) => {
                self.known.insert(gvk, Some(schema));
            }
            Err(error) => {
                if error.kind() != ErrorKind::NotFound {
                    self.unavailable.push((gvk.clone(), error.to_string()));
                }
                self.known.insert(gvk, None);
            }
        }
    }
}

#[cfg(test)]
mod tests;
