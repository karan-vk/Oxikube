//! What the diff tab shows once a comparison is done: [`DiffData`], built off the UI thread.

use oxikube_app::diff::ResourceDiff;

use crate::diff::{ManifestDiff, to_yaml};

/// A finished comparison, ready to draw: the app's result (paths, keys) and the line diff of the
/// two normalised objects written by the editor's one YAML writer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffData {
    /// The comparison: both sides, the changed paths, the keys.
    pub diff: ResourceDiff,
    /// The line diff of the two sides' text; a side that does not exist is empty text, so the
    /// other side reads as all added.
    pub lines: ManifestDiff,
    /// What the summary block lists: [`ResourceDiff::summary_lines`], then, for a ConfigMap or
    /// Secret, each key as `same`, `changed` or only on one side ([`ResourceDiff::key_lines`]).
    pub summary: Vec<String>,
}

impl DiffData {
    /// Writes both sides as YAML (keys sorted, Secret values as fingerprints) and diffs the
    /// lines. Run it off the UI thread: a 5 MB object takes a while, bounded by the line diff's
    /// own timeout.
    pub fn new(diff: ResourceDiff) -> Self {
        let text = |side: &oxikube_app::diff::DiffSide| {
            side.object.as_ref().map(to_yaml).unwrap_or_default()
        };
        let lines = ManifestDiff::between(&text(&diff.left), &text(&diff.right));
        let mut summary = diff.summary_lines();
        let keys = diff.key_lines();
        if !keys.is_empty() {
            summary.push(String::new());
            summary.push("Keys".to_owned());
            summary.extend(keys);
        }
        Self {
            diff,
            lines,
            summary,
        }
    }
}

/// What the tab is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffState {
    /// Reading or connecting; the text says which.
    Working(String),
    /// The comparison failed; the text says why (an error message, never an object).
    Failed(String),
    /// Done.
    Ready(std::sync::Arc<DiffData>),
}
