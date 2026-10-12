//! What the guard says about a bulk command (`selection::*`, E12-S19): the confirmation text and
//! the audit detail. Names and counts only: never a label value, never an annotation value.

use oxikube_domain::command::{Command, Propagation};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::selection::{SelectionSummary, edit_counts};

/// How many kinds the confirmation lists before it folds the rest.
const MAX_LINES: usize = 6;

/// The one-line confirmation of a bulk command, `None` for any other command.
pub(super) fn summary(command: &Command, context: &str) -> Option<String> {
    let what = |targets: &[ResourceRef]| {
        let counts = SelectionSummary::of(targets);
        let objects = match counts.total() {
            1 => "1 object".to_owned(),
            n => format!("{n} objects"),
        };
        (objects, counts.lines(MAX_LINES).join("; "))
    };
    Some(match command {
        Command::SelectionDelete {
            targets,
            propagation,
        } => {
            let (objects, lines) = what(targets);
            let cascade = match propagation {
                Propagation::Background => "",
                Propagation::Foreground => " Dependents are deleted first.",
                Propagation::Orphan => " Dependents are left in place.",
            };
            format!(
                "Delete {objects} on {context}: {lines}.{cascade} Each object is deleted and \
                 audited on its own."
            )
        }
        Command::SelectionRestart { targets } => {
            let (objects, lines) = what(targets);
            format!(
                "Restart {objects} on {context}: {lines}. Their pods are replaced by new ones in \
                 rolling updates, as `kubectl rollout restart` does."
            )
        }
        Command::SelectionLabel {
            targets,
            field,
            edits,
        } => {
            let (objects, lines) = what(targets);
            let (sets, removes) = edit_counts(edits);
            format!(
                "Change {sets} and remove {removes} {}{} on {objects} on {context}: {lines}.",
                field.noun(),
                if sets + removes == 1 { "" } else { "s" },
            )
        }
        _ => return None,
    })
}

/// What the audit record of a bulk command says besides its target (the cluster): how many
/// objects and of which kinds, as `items=5 kinds=Pod:3,ConfigMap:2`. For a label edit, the field
/// and how many keys it sets and removes, never the keys' values. The handler adds the batch id.
pub(super) fn detail(command: &Command) -> Option<String> {
    let targets = command.targets();
    if targets.is_empty() {
        return None;
    }
    let counts = SelectionSummary::of(targets);
    let kinds: Vec<String> = counts
        .kinds()
        .into_iter()
        .filter(|(kind, _)| kind.len() <= 40 && kind.chars().all(|c| c.is_ascii_alphanumeric()))
        .take(16)
        .map(|(kind, n)| format!("{kind}:{n}"))
        .collect();
    let mut detail = format!("items={} kinds={}", counts.total(), kinds.join(","));
    if let Command::SelectionLabel { field, edits, .. } = command {
        let (sets, removes) = edit_counts(edits);
        detail.push_str(&format!(
            " field={} set={sets} remove={removes}",
            field.key()
        ));
    }
    Some(detail)
}
