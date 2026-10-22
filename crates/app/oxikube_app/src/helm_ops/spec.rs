//! What the guard says about `helm::Rollback` and `helm::Uninstall`.

use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};

use crate::helm_install::release_gvk;

/// The confirmation text: the release, the revision or the flag, the namespace and the cluster.
pub(crate) fn summary(command: &Command, context: &str) -> Option<String> {
    match command {
        Command::HelmRollback {
            namespace,
            release,
            revision,
            ..
        } => Some(format!(
            "Roll back release {release} in namespace {namespace} to revision {revision} on \
             {context}"
        )),
        Command::HelmUninstall {
            namespace,
            release,
            keep_history,
            ..
        } => {
            let history = if *keep_history {
                "keeping its history"
            } else {
                "deleting its history"
            };
            Some(format!(
                "Uninstall release {release} from namespace {namespace} on {context}, {history}"
            ))
        }
        _ => None,
    }
}

/// The audit target: the release, as `helm.sh/v3 Release <namespace>/<name>`.
pub(crate) fn audit_target(command: &Command, cluster: &ClusterId) -> Option<ResourceRef> {
    let (Command::HelmRollback {
        namespace, release, ..
    }
    | Command::HelmUninstall {
        namespace, release, ..
    }) = command
    else {
        return None;
    };
    Some(ResourceRef::namespaced(
        cluster.clone(),
        release_gvk(),
        namespace.as_str(),
        release.as_str(),
    ))
}

/// The audit detail: the revision (rollback) or `keep_history` (uninstall). Nothing else.
pub(crate) fn audit_detail(command: &Command) -> Option<String> {
    match command {
        Command::HelmRollback { revision, .. } => Some(format!("revision={revision}")),
        Command::HelmUninstall { keep_history, .. } => Some(format!("keep_history={keep_history}")),
        _ => None,
    }
}

/// The text a type-the-name confirmation of these commands must repeat: the release name.
pub(crate) fn expected_name(command: &Command) -> Option<String> {
    match command {
        Command::HelmRollback { release, .. } | Command::HelmUninstall { release, .. } => {
            Some(release.clone())
        }
        _ => None,
    }
}
