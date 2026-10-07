//! Why a followed pod's stream ended: [`PodIdentity`] (what was followed) and [`PodFate`] (what
//! became of it), read through the [`ResourceReader`].

use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::view::PodPhase;
use oxikube_domain::{OwnerRef, OxiResult, Resource};
use oxikube_ports::ResourceReader;

use crate::logs::EndReason;

/// The `v1` Pod kind.
pub(crate) fn pod_kind() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// The pod a session followed, as it was when the stream opened: enough to find its replacement
/// after it is gone (its controller and node outlive it here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodIdentity {
    /// Namespace of the pod.
    pub namespace: String,
    /// Pod name.
    pub name: String,
    /// `metadata.uid`: a pod recreated under the same name (a StatefulSet's) has another one.
    pub uid: Option<Arc<str>>,
    /// The controller that owns the pod (a ReplicaSet, StatefulSet, DaemonSet, Job, ...), which
    /// makes a replacement; `None` for a bare pod.
    pub controller: Option<OwnerRef>,
    /// `spec.nodeName`: a DaemonSet's replacement runs on the same node.
    pub node: Option<String>,
}

impl PodIdentity {
    /// The identity of `pod`.
    pub fn of(pod: &Resource) -> Self {
        Self {
            namespace: pod.namespace().unwrap_or_default().to_owned(),
            name: pod.name().to_owned(),
            uid: pod.meta.uid.clone(),
            controller: pod.meta.controller_ref().cloned(),
            node: pod.get_str("/spec/nodeName").map(str::to_owned),
        }
    }

    /// Whether `pod` is this pod (same name and, when known, the same uid).
    pub(crate) fn is(&self, pod: &Resource) -> bool {
        pod.name() == self.name && (self.uid.is_none() || pod.meta.uid == self.uid)
    }
}

/// What became of a followed pod after its stream closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PodFate {
    /// It still runs: the stream closed for another reason (a connection, a timeout).
    Running,
    /// It ran to its end (`Succeeded` or `Failed`).
    Finished,
    /// It is gone or going: deleted, terminating, or replaced by a pod of the same name.
    Gone,
}

impl PodFate {
    /// The end reason of a pod's session for this fate (`None` while it runs).
    pub(crate) fn end_reason(self, identity: Option<&PodIdentity>) -> Option<EndReason> {
        match self {
            Self::Running => None,
            Self::Finished => Some(EndReason::PodFinished),
            Self::Gone if identity.is_some_and(|i| i.controller.is_some()) => {
                Some(EndReason::PodReplaced)
            }
            Self::Gone => Some(EndReason::PodDeleted),
        }
    }
}

/// Reads the pod `name` of `namespace` (`None` when it does not exist).
pub(crate) async fn read_pod(
    resources: &dyn ResourceReader,
    namespace: &str,
    name: &str,
) -> OxiResult<Option<Resource>> {
    resources.get_opt(&pod_kind(), Some(namespace), name).await
}

/// What became of the pod `identity` names (`name` alone when its identity was never read).
pub(crate) async fn fate(
    resources: &dyn ResourceReader,
    namespace: &str,
    name: &str,
    identity: Option<&PodIdentity>,
) -> OxiResult<PodFate> {
    let Some(pod) = read_pod(resources, namespace, name).await? else {
        return Ok(PodFate::Gone);
    };
    Ok(fate_of(&pod, identity))
}

/// What `pod` (the object now under the followed pod's name) says became of it.
pub(crate) fn fate_of(pod: &Resource, identity: Option<&PodIdentity>) -> PodFate {
    if identity.is_some_and(|i| !i.is(pod)) || pod.meta.is_terminating() {
        return PodFate::Gone;
    }
    let phase = PodPhase::parse(pod.get_str("/status/phase"));
    if phase.is_terminal() {
        PodFate::Finished
    } else {
        PodFate::Running
    }
}

#[cfg(test)]
mod tests {
    use oxikube_testkit::pod;

    use super::*;

    fn owned(uid: &str) -> Resource {
        let mut json = pod().name("web-0").namespace("default").uid(uid).json();
        json["metadata"]["ownerReferences"] = serde_json::json!([{
            "apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web-5d8",
            "uid": "rs-1", "controller": true
        }]);
        json["spec"]["nodeName"] = serde_json::json!("node-a");
        Resource::from_json(json).unwrap()
    }

    #[test]
    fn the_identity_keeps_the_controller_and_the_node() {
        let identity = PodIdentity::of(&owned("u1"));
        assert_eq!(identity.name, "web-0");
        assert_eq!(identity.node.as_deref(), Some("node-a"));
        assert_eq!(
            identity.controller.as_ref().map(|c| &*c.kind),
            Some("ReplicaSet")
        );
    }

    #[test]
    fn running_finished_terminating_and_recreated_pods() {
        let identity = PodIdentity::of(&owned("u1"));
        assert_eq!(fate_of(&owned("u1"), Some(&identity)), PodFate::Running);
        assert_eq!(
            fate_of(&owned("u2"), Some(&identity)),
            PodFate::Gone,
            "a new uid"
        );
        let done = pod().name("web-0").uid("u1").succeeded().build();
        assert_eq!(fate_of(&done, Some(&identity)), PodFate::Finished);
        let crashed = pod().name("web-0").uid("u1").failed().build();
        assert_eq!(fate_of(&crashed, None), PodFate::Finished);
        let going = pod().name("web-0").uid("u1").terminating().build();
        assert_eq!(fate_of(&going, Some(&identity)), PodFate::Gone);
    }

    #[test]
    fn a_gone_pod_with_a_controller_is_replaced_and_a_bare_one_deleted() {
        let owned = PodIdentity::of(&owned("u1"));
        let bare = PodIdentity::of(&pod().name("solo").uid("u9").build());
        assert_eq!(
            PodFate::Gone.end_reason(Some(&owned)),
            Some(EndReason::PodReplaced)
        );
        assert_eq!(
            PodFate::Gone.end_reason(Some(&bare)),
            Some(EndReason::PodDeleted)
        );
        assert_eq!(PodFate::Gone.end_reason(None), Some(EndReason::PodDeleted));
        assert_eq!(
            PodFate::Finished.end_reason(Some(&owned)),
            Some(EndReason::PodFinished)
        );
        assert_eq!(PodFate::Running.end_reason(Some(&owned)), None);
    }
}
