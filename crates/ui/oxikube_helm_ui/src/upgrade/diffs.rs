//! The two diffs the tab shows: the edited values against the release's current ones, and the
//! chart's default values at the current version against the target version. Both are line diffs
//! of the E10 editor (`oxikube_editor::diff::ManifestDiff`) over text with secret-looking values
//! fingerprinted ([`mask_values_for_diff`]), computed on the background executor.

use oxikube_domain::helm::mask_values_for_diff;
use oxikube_editor::diff::ManifestDiff;

/// The diff of the release's `current` values to the `edited` ones. Secret-looking values
/// (`password`, `token`, ...) show as fingerprints: a changed secret reads as a change, its value
/// never appears.
pub fn values_diff(current: &str, edited: &str) -> ManifestDiff {
    ManifestDiff::between(
        &mask_values_for_diff(current),
        &mask_values_for_diff(edited),
    )
}

/// The diff of the chart's default values at the version the release runs to those of the target
/// version: the keys the upgrade adds, renames or drops.
pub fn defaults_diff(from: &str, to: &str) -> ManifestDiff {
    values_diff(from, to)
}
