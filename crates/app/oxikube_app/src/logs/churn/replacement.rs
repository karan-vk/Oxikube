//! [`find_replacement`]: the pod that took over from a followed pod that is gone (the viewer's
//! "follow replacement", `logs::FollowReplacement`).
//!
//! The pod's controller says where its replacement comes from. A ReplicaSet's pods are replaced by
//! its Deployment's (a rollout makes a new ReplicaSet, so the Deployment's selector is the one that
//! finds the new pod); a StatefulSet recreates the pod under the same name; a DaemonSet's
//! replacement runs on the same node; a Job's retry is a new pod of the Job. The candidates are
//! the pods the controller's selector matches now, minus the gone pod and pods that are going
//! too, and only those that can have replaced it: a StatefulSet's namesake, a DaemonSet's pod on
//! the same node, otherwise a pod made after the gone one (an older sibling, such as a replica
//! left after a scale-down, is not a replacement). Of those, one still running, then the newest.

use std::cmp::Reverse;

use oxikube_domain::view::PodPhase;
use oxikube_domain::{OwnerRef, OxiResult, Resource};
use oxikube_ports::{ListOptions, ResourceReader};

use super::probe::{PodIdentity, pod_kind, read_pod};
use crate::logs::selector_of;

/// The name of the pod that replaced `gone`, `None` when there is none (yet): the controller is
/// gone too, it has not made a new pod, or a bare pod was deleted.
///
/// # Errors
///
/// A failure to read the controller or list the pods (`Forbidden`, `Network`, ...).
pub async fn find_replacement(
    resources: &dyn ResourceReader,
    gone: &PodIdentity,
) -> OxiResult<Option<String>> {
    let namespace = gone.namespace.as_str();
    let Some(owner) = &gone.controller else {
        // A bare pod has no controller; a pod recreated under its name (by hand) still counts.
        let pod = read_pod(resources, namespace, &gone.name).await?;
        return Ok(pod
            .filter(|pod| is_candidate(pod, gone))
            .map(|pod| pod.name().to_owned()));
    };
    let pods = match top_controller(resources, namespace, owner).await? {
        None => return Ok(None),
        Some(top) => match selector_of(&top) {
            Ok(selector) => list_pods(resources, namespace, Some(selector)).await?,
            // An owner whose selector this cannot read (a custom controller): its own pods.
            Err(_) => list_pods(resources, namespace, None)
                .await?
                .into_iter()
                .filter(|pod| {
                    pod.meta
                        .controller_ref()
                        .is_some_and(|o| o.uid == owner.uid)
                })
                .collect(),
        },
    };
    Ok(pods
        .iter()
        .filter(|pod| is_candidate(pod, gone) && replaces(pod, gone, owner))
        .max_by_key(|pod| {
            let running = !PodPhase::parse(pod.get_str("/status/phase")).is_terminal();
            (running, pod.meta.creation, Reverse(pod.name().to_owned()))
        })
        .map(|pod| pod.name().to_owned()))
}

/// Whether `pod` may replace `gone`: another pod (or the same name with a new uid) that is not
/// going away itself.
fn is_candidate(pod: &Resource, gone: &PodIdentity) -> bool {
    !gone.is(pod) && !pod.meta.is_terminating()
}

/// Whether `owner` (the gone pod's controller) made `pod` in place of `gone`, rather than it being
/// a sibling that was already there: a StatefulSet recreates the same name, a DaemonSet runs the
/// new pod on the same node, any other controller makes a pod newer than the gone one. A fact the
/// gone pod did not record (its node, its creation time) does not rule a pod out.
fn replaces(pod: &Resource, gone: &PodIdentity, owner: &OwnerRef) -> bool {
    match &*owner.kind {
        "StatefulSet" => pod.name() == gone.name,
        "DaemonSet" => gone
            .node
            .as_deref()
            .is_none_or(|node| pod.get_str("/spec/nodeName") == Some(node)),
        _ => match (gone.created, pod.meta.creation) {
            (Some(gone), Some(created)) => created > gone,
            _ => true,
        },
    }
}

/// The object whose selector finds the replacement: the Deployment of a ReplicaSet that has one,
/// else the owner itself. `None` when it no longer exists.
async fn top_controller(
    resources: &dyn ResourceReader,
    namespace: &str,
    owner: &OwnerRef,
) -> OxiResult<Option<Resource>> {
    let Some(object) = resources
        .get_opt(&owner.gvk(), Some(namespace), &owner.name)
        .await?
    else {
        return Ok(None);
    };
    if &*owner.kind != "ReplicaSet" {
        return Ok(Some(object));
    }
    match object.meta.controller_ref() {
        Some(parent) if &*parent.kind == "Deployment" => {
            let deployment = resources
                .get_opt(&parent.gvk(), Some(namespace), &parent.name)
                .await?;
            Ok(deployment.or(Some(object)))
        }
        _ => Ok(Some(object)),
    }
}

async fn list_pods(
    resources: &dyn ResourceReader,
    namespace: &str,
    selector: Option<String>,
) -> OxiResult<Vec<Resource>> {
    let mut options = ListOptions::default();
    if let Some(selector) = selector {
        options = options.labels(selector);
    }
    Ok(resources
        .list(&pod_kind(), Some(namespace), &options)
        .await?
        .items)
}
