//! Which pods a drain evicts, skips or refuses: kubectl's drain filters, as a pure function.

use oxikube_domain::Resource;
use oxikube_domain::json::JsonRef;

use super::options::{
    BlockReason, BlockedPod, DrainOptions, DrainPlan, PodRef, SkipReason, SkippedPod,
};

/// The annotation the kubelet puts on the mirror pod of a static pod.
const MIRROR_ANNOTATION: &str = "kubernetes.io/config.mirror";

/// Sorts `pods` (the pods on one node) into a [`DrainPlan`].
///
/// Per pod, first match wins:
///
/// 1. a mirror pod is skipped: the kubelet owns it;
/// 2. a DaemonSet-managed pod is skipped with `ignore_daemonsets`, otherwise it blocks;
/// 3. a pod that already finished (`Succeeded` or `Failed`) is evicted, whatever else is true
///    of it: nothing is lost by deleting it;
/// 4. a pod with no controller blocks unless `force`;
/// 5. a pod with an `emptyDir` volume blocks unless `delete_emptydir_data`;
/// 6. every other pod, terminating ones included (the eviction is accepted and the drain then
///    waits for it), is evicted.
pub fn plan_drain(pods: &[Resource], options: &DrainOptions) -> DrainPlan {
    let mut plan = DrainPlan::default();
    for pod in pods {
        let reference = pod_ref(pod);
        match classify(pod, options) {
            Verdict::Evict => plan.evict.push(reference),
            Verdict::Skip(reason) => plan.skipped.push(SkippedPod {
                pod: reference,
                reason,
            }),
            Verdict::Block(reason) => plan.blocked.push(BlockedPod {
                pod: reference,
                reason,
            }),
        }
    }
    plan
}

enum Verdict {
    Evict,
    Skip(SkipReason),
    Block(BlockReason),
}

fn classify(pod: &Resource, options: &DrainOptions) -> Verdict {
    if pod.meta.annotations.contains_key(MIRROR_ANNOTATION) {
        return Verdict::Skip(SkipReason::Mirror);
    }
    let controller = pod.meta.controller_ref();
    if controller.is_some_and(|owner| &*owner.kind == "DaemonSet") {
        return if options.ignore_daemonsets {
            Verdict::Skip(SkipReason::DaemonSet)
        } else {
            Verdict::Block(BlockReason::DaemonSet)
        };
    }
    if matches!(pod.get_str("/status/phase"), Some("Succeeded" | "Failed")) {
        return Verdict::Evict;
    }
    if controller.is_none() && !options.force {
        return Verdict::Block(BlockReason::Unmanaged);
    }
    if uses_empty_dir(pod) && !options.delete_emptydir_data {
        return Verdict::Block(BlockReason::LocalStorage);
    }
    Verdict::Evict
}

fn uses_empty_dir(pod: &Resource) -> bool {
    pod.get("/spec/volumes")
        .and_then(JsonRef::as_array)
        .is_some_and(|volumes| volumes.iter().any(|v| v.get("emptyDir").is_some()))
}

fn pod_ref(pod: &Resource) -> PodRef {
    PodRef {
        namespace: pod.namespace().unwrap_or_default().to_owned(),
        name: pod.name().to_owned(),
        uid: pod.meta.uid.as_deref().map(str::to_owned),
    }
}
