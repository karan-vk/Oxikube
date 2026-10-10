//! The notice for an editor setting that was rejected: a warning toast naming each key and the
//! default used instead (never the value from the file).

use gpui::App;
use oxikube_settings::SettingsStore;
use oxikube_workspace::Toast;

use super::EditorSettings;

/// The deduplication key of the toast: a new problem list replaces the one on screen.
const NOTICE_KEY: &str = "editor-settings";

/// The problems of the global `editor` settings and of every cluster's override, sorted, each
/// once. Empty when every value is valid (or there is no settings store).
pub fn problems(cx: &App) -> Vec<String> {
    let Some(store) = cx.try_global::<SettingsStore>() else {
        return Vec::new();
    };
    let mut all: Vec<String> = store
        .try_get::<EditorSettings>(None)
        .into_iter()
        .chain(store.cluster_values::<EditorSettings>().map(|(_, s)| s))
        .flat_map(|settings| settings.problems.iter().cloned())
        .collect();
    all.sort();
    all.dedup();
    all
}

/// The warning toast for `problems`, `None` when there are none.
pub fn notice(problems: &[String]) -> Option<Toast> {
    if problems.is_empty() {
        return None;
    }
    Some(
        Toast::warning(problems.join("\n"))
            .title("Editor settings")
            .key(NOTICE_KEY),
    )
}
