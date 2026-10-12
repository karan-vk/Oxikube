//! A revision as a table row, computed once when the history arrives.

use gpui::SharedString;
use jiff::Timestamp;
use oxikube_domain::Age;
use oxikube_domain::rollout::{Revision, TemplateChange};

/// One row of the history table: everything drawn, as text, so a frame formats nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    /// The revision number.
    pub revision: i64,
    /// The ReplicaSet's or ControllerRevision's name.
    pub name: SharedString,
    /// The change cause, or `-` when none was recorded.
    pub cause: SharedString,
    /// The images, joined with `, `.
    pub images: SharedString,
    /// `2/2` ready of the revision's ReplicaSet; empty for a ControllerRevision.
    pub ready: SharedString,
    /// The age at the time the history was read (`3d`), empty when unknown.
    pub age: SharedString,
    /// Whether this is the running revision.
    pub current: bool,
    /// How the revision differs from the running template.
    pub changes: Vec<TemplateChange>,
    /// The preview under the table when this row is chosen: one line per change, or one line
    /// saying there is none.
    pub preview: Vec<SharedString>,
    /// The heading over the preview.
    pub preview_title: SharedString,
}

impl HistoryRow {
    /// The row of `revision`, with ages measured at `now`. Every string the dialog draws is made
    /// here, once, so a frame only clones handles.
    pub fn of(revision: &Revision, now: Timestamp) -> Self {
        let preview = if revision.current {
            vec!["This is the running revision.".into()]
        } else if revision.changes.is_empty() {
            vec!["The pod template is the same as the running one.".into()]
        } else {
            revision
                .changes
                .iter()
                .map(|change| change_line(change).into())
                .collect()
        };
        Self {
            revision: revision.revision,
            name: revision.name.to_string().into(),
            cause: revision
                .change_cause
                .clone()
                .unwrap_or_else(|| "-".to_owned())
                .into(),
            images: revision.images.join(", ").into(),
            ready: revision
                .replicas
                .map(|r| format!("{}/{}", r.ready, r.desired))
                .unwrap_or_default()
                .into(),
            age: revision
                .created
                .map(|created| Age::between(created, now).to_kubectl_string())
                .unwrap_or_default()
                .into(),
            current: revision.current,
            changes: revision.changes.clone(),
            preview,
            preview_title: format!(
                "REVISION {} COMPARED WITH THE RUNNING TEMPLATE",
                revision.revision
            )
            .into(),
        }
    }
}

/// One preview line: `spec.containers[web].image: nginx:1.0 -> nginx:1.1`.
fn change_line(change: &TemplateChange) -> String {
    format!(
        "{}: {} -> {}",
        change.path,
        change.from.as_deref().unwrap_or("(none)"),
        change.to.as_deref().unwrap_or("(none)")
    )
}
