//! [`ManifestModel`]: what the manifest editor decides, written against [`EditorApi`] only, so
//! it is tested without a window (`tests` below use a plain struct as the editor).
//!
//! It keeps the schemas the buffer's kinds need (which to fetch, which arrived, which the cluster
//! does not have), drops a validation result computed for an older buffer version and counts the
//! problems the toolbar shows.

use std::collections::HashSet;
use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ui::editor::{DiagnosticLevel, EditorApi};

use super::validation::{KnownSchemas, Validation};

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
    /// The buffer changed since: the result was dropped.
    Stale,
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

    /// The buffer version whose diagnostics are shown, if any.
    pub fn shown_version(&self) -> Option<u64> {
        self.shown
    }

    /// Kinds whose schema could not be fetched (not "the cluster has none"), with the reason.
    pub fn unavailable(&self) -> &[(Gvk, String)] {
        &self.unavailable
    }

    /// The buffer changed: the shown diagnostics are gone (the editor drops them on an edit).
    pub fn changed(&mut self) {
        self.problems = Problems::default();
        self.shown = None;
    }

    /// Shows `result` when it is for the buffer's current version; drops it otherwise.
    pub fn accept(&mut self, editor: &mut dyn EditorApi, result: Validation) -> Accepted {
        if result.version != editor.version() {
            return Accepted::Stale;
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
        editor.set_diagnostics(result.diagnostics);
        Accepted::Shown
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
