//! The sentences of the review, as pure functions of the preview (tested without a window).

use oxikube_app::apply::Change;
use oxikube_app::{CopyError, CopyPreview};

/// How many dropped fields the review lists before it folds the rest.
pub const MAX_REMOVED_SHOWN: usize = 8;

/// What the copy does at the target: "Creates Deployment prod/web on kind-b".
pub fn change_line(preview: &CopyPreview, cluster: &str) -> String {
    let label = preview.label();
    match preview.change() {
        Change::Create => format!("Creates {label} on {cluster}"),
        Change::Update => format!("{label} exists on {cluster}: overwriting changes it"),
        Change::Unchanged => format!("{label} on {cluster} is identical: nothing would change"),
    }
}

/// The heading over the dropped fields: "Dropped 7 server-owned fields". Paths only, never values.
pub fn removed_heading(removed: &[String]) -> String {
    match removed.len() {
        0 => "Nothing had to be dropped".to_owned(),
        1 => "Dropped 1 server-owned field".to_owned(),
        n => format!("Dropped {n} server-owned fields"),
    }
}

/// The dropped paths shown, and how many more there are.
pub fn removed_shown(removed: &[String]) -> (&[String], usize) {
    let shown = removed.len().min(MAX_REMOVED_SHOWN);
    (&removed[..shown], removed.len() - shown)
}

/// The sentence for a copy that failed or cannot start; names and fields, never values.
pub fn error_line(error: &CopyError) -> String {
    match error {
        CopyError::Exists(label) => {
            format!("{label} exists already: choose overwrite, skip or rename")
        }
        other => other.to_string(),
    }
}

/// The name suggested for a rename: `<name>-copy`, cut so it stays a valid name.
pub fn rename_suggestion(name: &str) -> String {
    const SUFFIX: &str = "-copy";
    let room = 63usize.saturating_sub(SUFFIX.len());
    let stem: String = name.chars().take(room).collect();
    format!("{}{SUFFIX}", stem.trim_end_matches(['-', '.']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_removed_fields_are_counted_and_folded() {
        let removed: Vec<String> = (0..11).map(|i| format!("metadata.f{i}")).collect();
        assert_eq!(removed_heading(&removed), "Dropped 11 server-owned fields");
        let (shown, more) = removed_shown(&removed);
        assert_eq!((shown.len(), more), (MAX_REMOVED_SHOWN, 3));
        assert_eq!(
            removed_heading(&removed[..1]),
            "Dropped 1 server-owned field"
        );
        assert_eq!(removed_heading(&[]), "Nothing had to be dropped");
    }

    #[test]
    fn a_rename_suggestion_appends_copy_and_stays_a_valid_length() {
        assert_eq!(rename_suggestion("web"), "web-copy");
        let long = "a".repeat(70);
        let suggestion = rename_suggestion(&long);
        assert_eq!(suggestion.len(), 63);
        assert!(suggestion.ends_with("-copy"));
        assert_eq!(rename_suggestion("web-"), "web-copy");
    }

    #[test]
    fn an_existing_object_error_names_the_three_ways_out() {
        let line = error_line(&CopyError::Exists("ConfigMap prod/web".into()));
        assert!(line.contains("overwrite, skip or rename"), "{line}");
    }
}
