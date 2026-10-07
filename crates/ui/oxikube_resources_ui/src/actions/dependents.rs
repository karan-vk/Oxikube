//! Which kinds own dependents, so the delete dialog asks how to treat them only where there are
//! some (E07-U563).

use oxikube_domain::access::is_builtin_api_group;
use oxikube_domain::ids::Gvk;

/// Whether deleting an object of `gvk` can leave dependents (objects whose owner reference points
/// at it) behind or take them along: the workload controllers, and any kind a CustomResource
/// Definition adds (an operator's resource may own anything). A Pod, a ConfigMap, a Secret and
/// the other built-in kinds own nothing, so their delete has no propagation to choose.
pub(super) fn owns_dependents(gvk: &Gvk) -> bool {
    let group: &str = &gvk.group;
    match (group, &*gvk.kind) {
        ("apps", "Deployment" | "ReplicaSet" | "StatefulSet" | "DaemonSet")
        | ("batch", "Job" | "CronJob")
        | ("", "ReplicationController") => true,
        _ => !is_builtin_api_group(group),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controllers_and_custom_resources_own_dependents_leaf_kinds_do_not() {
        for (group, kind) in [
            ("apps", "Deployment"),
            ("apps", "StatefulSet"),
            ("batch", "CronJob"),
            ("", "ReplicationController"),
            ("argoproj.io", "Rollout"),
        ] {
            assert!(owns_dependents(&Gvk::new(group, "v1", kind)), "{kind}");
        }
        for (group, kind) in [
            ("", "Pod"),
            ("", "ConfigMap"),
            ("", "Secret"),
            ("", "Node"),
            ("", "Service"),
            ("networking.k8s.io", "Ingress"),
            ("storage.k8s.io", "StorageClass"),
        ] {
            assert!(!owns_dependents(&Gvk::new(group, "v1", kind)), "{kind}");
        }
    }
}
