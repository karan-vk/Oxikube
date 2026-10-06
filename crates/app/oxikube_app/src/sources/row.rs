//! [`SourceRow`]: one line of the sources screen.

use std::path::Path;

use oxikube_ports::{
    ClusterSource, SourceKind, SourceState, SourceStatus, UserSource, UserSourceKind,
};

/// One entry of the user's source list with how reading it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRow {
    /// The list entry.
    pub source: UserSource,
    /// What to show as its name: the path, or a description of the default entry.
    pub label: String,
    /// Oxikube stored this file itself (it was pasted), so removing the row deletes the file.
    pub stored: bool,
    /// How the last read went; `None` before the source was read.
    pub state: Option<SourceState>,
    /// How many contexts the source gave.
    pub contexts: usize,
    /// Why the source is not fully usable, or a note about it. Fixed wording, no file content.
    pub message: Option<String>,
}

impl SourceRow {
    /// Whether reading the source went wrong: missing, unreadable or not a kubeconfig. A blank
    /// source is a note, not an error.
    pub fn is_error(&self) -> bool {
        matches!(
            self.state,
            Some(SourceState::Missing | SourceState::Unreadable | SourceState::Invalid)
        )
    }
}

/// The label of a list entry.
pub(super) fn label(source: &UserSource) -> String {
    match (&source.kind, &source.path) {
        (UserSourceKind::Default, _) => "KUBECONFIG or ~/.kube/config".to_owned(),
        (_, Some(path)) => path.display().to_string(),
        (_, None) => String::new(),
    }
}

/// Whether `status` belongs to the list entry `source`. The default entry owns the statuses of
/// the kubectl tier (`KUBECONFIG` files or the default path); a file or directory owns the one
/// status of its path.
fn belongs(source: &UserSource, status: &ClusterSource) -> bool {
    match source.kind {
        UserSourceKind::Default => {
            status.kind == SourceKind::Environment || status.id.0 == "default"
        }
        _ => {
            source
                .path
                .as_deref()
                .is_some_and(|p| Some(p) == status.path.as_deref())
                && status.kind != SourceKind::Environment
                && status.id.0 != "default"
        }
    }
}

/// Builds the row of `source` from `statuses`.
pub(super) fn build(source: &UserSource, stored: bool, statuses: &[SourceStatus]) -> SourceRow {
    let mine: Vec<&SourceStatus> = statuses
        .iter()
        .filter(|s| belongs(source, &s.source))
        .collect();
    let contexts = mine.iter().map(|s| s.contexts).sum();
    // The worst problem decides, unless something was found: a missing default file next to a
    // working `KUBECONFIG` entry is not worth an error.
    let (state, message) = if mine.is_empty() {
        (None, None)
    } else if mine.iter().any(|s| s.state == SourceState::Found) {
        let note = mine.iter().find_map(|s| s.message.clone());
        (Some(SourceState::Found), note)
    } else {
        let first = mine[0];
        (Some(first.state), first.message.clone())
    };
    SourceRow {
        source: source.clone(),
        label: label(source),
        stored,
        state,
        contexts,
        message,
    }
}

/// The path of a file or directory entry.
pub(super) fn path_of(source: &UserSource) -> Option<&Path> {
    source.path.as_deref()
}
