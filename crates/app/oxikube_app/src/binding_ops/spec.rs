//! What the guard says about `binding::Create`, `binding::Edit` and `binding::Delete`.

use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};

use super::BindingManifest;
use crate::actions::object_label;

/// The confirmation text: the binding, the role, the cluster, and a warning for a grant to
/// `cluster-admin` or to everybody.
pub(crate) fn summary(command: &Command, context: &str) -> Option<String> {
    match command {
        Command::BindingCreate { manifest, .. } => {
            let binding = BindingManifest::parse(manifest).ok()?;
            Some(with_warnings(
                format!(
                    "Create {} {} granting {} to {} on {context}. Changing RBAC can grant \
                     privileges.",
                    binding.kind.as_str(),
                    binding.label(),
                    binding.role_label(),
                    subjects_word(binding.subjects.len()),
                ),
                &binding,
            ))
        }
        Command::BindingEdit {
            manifest, previous, ..
        } => {
            let binding = BindingManifest::parse(manifest).ok()?;
            let before = previous
                .as_deref()
                .and_then(|text| BindingManifest::parse(text).ok());
            let text = match before {
                Some(before) => format!(
                    "Change {} {} from {} to {} for {} on {context}. The API cannot change a \
                     roleRef, so the binding is deleted and created again as one step.",
                    binding.kind.as_str(),
                    binding.label(),
                    before.role_label(),
                    binding.role_label(),
                    subjects_word(binding.subjects.len()),
                ),
                None => format!(
                    "Replace the subjects of {} {} with {} (granting {}) on {context}. \
                     Changing RBAC can grant privileges.",
                    binding.kind.as_str(),
                    binding.label(),
                    subjects_word(binding.subjects.len()),
                    binding.role_label(),
                ),
            };
            Some(with_warnings(text, &binding))
        }
        Command::BindingDelete { target } => Some(format!(
            "Delete {} on {context}. Its subjects lose what it grants.",
            object_label(target)
        )),
        _ => None,
    }
}

/// The audit target: the binding, from the manifest.
pub(crate) fn audit_target(command: &Command, cluster: &ClusterId) -> Option<ResourceRef> {
    match command {
        Command::BindingCreate { manifest, .. } | Command::BindingEdit { manifest, .. } => {
            Some(BindingManifest::parse(manifest).ok()?.target(cluster))
        }
        _ => None,
    }
}

/// The audit detail: the role and how many subjects, and that a `roleRef` change was made as a
/// delete and a create. Never the subjects' names.
pub(crate) fn audit_detail(command: &Command) -> Option<String> {
    match command {
        Command::BindingCreate { manifest, .. } => detail(manifest, None),
        Command::BindingEdit {
            manifest, previous, ..
        } => detail(manifest, Some(previous.is_some())),
        _ => None,
    }
}

fn detail(manifest: &str, recreate: Option<bool>) -> Option<String> {
    let binding = BindingManifest::parse(manifest).ok()?;
    let mut text = format!(
        "role={} subjects={}",
        binding.role_label(),
        binding.subjects.len()
    );
    if recreate == Some(true) {
        text.push_str(" recreate=delete+create");
    }
    Some(text)
}

fn with_warnings(mut text: String, binding: &BindingManifest) -> String {
    for escalation in binding.escalations() {
        text.push_str(" WARNING: ");
        text.push_str(escalation.message());
    }
    text
}

fn subjects_word(count: usize) -> String {
    if count == 1 {
        "1 subject".to_owned()
    } else {
        format!("{count} subjects")
    }
}
