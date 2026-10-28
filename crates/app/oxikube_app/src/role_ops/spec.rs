//! What the guard says about `role::Create`, `role::Edit` and `role::Delete`.

use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};

use super::RoleManifest;
use crate::actions::object_label;

/// The confirmation text: the role, the cluster, and for a delete the bindings it leaves behind.
pub(crate) fn summary(command: &Command, context: &str) -> Option<String> {
    match command {
        Command::RoleCreate { manifest, .. } => {
            let role = RoleManifest::parse(manifest).ok()?;
            Some(format!(
                "Create {} {} with {} on {context}. Changing RBAC can grant privileges.",
                role.kind.as_str(),
                label(&role),
                rules_word(role.rules)
            ))
        }
        Command::RoleEdit { manifest, .. } => {
            let role = RoleManifest::parse(manifest).ok()?;
            Some(format!(
                "Replace the rules of {} {} with {} on {context}. Changing RBAC can grant \
                 privileges.",
                role.kind.as_str(),
                label(&role),
                rules_word(role.rules)
            ))
        }
        Command::RoleDelete { target, bindings } => {
            let object = object_label(target);
            Some(match bindings {
                Some(0) | None => format!("Delete {object} on {context}"),
                Some(count) => format!(
                    "Delete {object} on {context}. {count} {} name it and will grant nothing \
                     afterwards.",
                    if *count == 1 { "binding" } else { "bindings" }
                ),
            })
        }
        _ => None,
    }
}

/// The audit target: the role, from the manifest (`rbac.authorization.k8s.io Role ns/name`).
pub(crate) fn audit_target(command: &Command, cluster: &ClusterId) -> Option<ResourceRef> {
    match command {
        Command::RoleCreate { manifest, .. } | Command::RoleEdit { manifest, .. } => {
            Some(RoleManifest::parse(manifest).ok()?.target(cluster))
        }
        _ => None,
    }
}

/// The audit detail: how many rules a create or edit holds, and how many bindings a delete left.
/// Never the rules themselves.
pub(crate) fn audit_detail(command: &Command) -> Option<String> {
    match command {
        Command::RoleCreate { manifest, .. } | Command::RoleEdit { manifest, .. } => {
            let role = RoleManifest::parse(manifest).ok()?;
            Some(format!("rules={}", role.rules))
        }
        Command::RoleDelete { bindings, .. } => bindings.map(|count| format!("bindings={count}")),
        _ => None,
    }
}

fn label(role: &RoleManifest) -> String {
    match &role.namespace {
        Some(namespace) => format!("{namespace}/{}", role.name),
        None => role.name.clone(),
    }
}

fn rules_word(count: usize) -> String {
    if count == 1 {
        "1 rule".to_owned()
    } else {
        format!("{count} rules")
    }
}
