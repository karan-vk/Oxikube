//! What the followed container's status says once its stream closed in a pod that still runs:
//! it runs (the connection dropped), it is done for good (an init container that completed, a
//! container that exited and will not restart), or it is between restarts (`CrashLoopBackOff`).
//!
//! The pod's phase alone cannot tell: a completed init container, or a container that exited
//! next to a live sidecar, leaves the pod `Pending` or `Running`.

use oxikube_domain::Resource;
use oxikube_domain::view::{ContainerKind, ContainerState, ContainerSummary};

/// `metadata.annotations` key naming the container a request without one reads.
const DEFAULT_CONTAINER: &str = "kubectl.kubernetes.io/default-container";

/// What became of the followed container in a pod that still runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerFate {
    /// It runs, or its status says nothing more: the stream closed for another reason.
    Running,
    /// It exited and will not run again: its log is complete.
    Finished,
    /// It exited or waits to run again (`CrashLoopBackOff`, a restart under way): the next
    /// instance's log comes when it starts.
    Restarting,
}

/// The fate of the container `name` of `pod` (`None`: the container a request without a name
/// reads, resolved as the API server does). A container that cannot be told is `Running`.
pub(crate) fn container_fate(pod: &Resource, name: Option<&str>) -> ContainerFate {
    let Ok(containers) = ContainerSummary::list_from_resource(pod) else {
        return ContainerFate::Running;
    };
    let Some(container) = followed(pod, &containers, name) else {
        return ContainerFate::Running;
    };
    match &container.state {
        ContainerState::Waiting { .. } => ContainerFate::Restarting,
        ContainerState::Terminated(end) if will_restart(pod, container.kind, end.exit_code) => {
            ContainerFate::Restarting
        }
        ContainerState::Terminated(_) => ContainerFate::Finished,
        ContainerState::Running { .. } | ContainerState::Unknown => ContainerFate::Running,
    }
}

/// The container a read of `name` follows: `name`, else the default-container annotation, else
/// the pod's only regular container.
fn followed<'a>(
    pod: &Resource,
    containers: &'a [ContainerSummary],
    name: Option<&str>,
) -> Option<&'a ContainerSummary> {
    let find = |n: &str| containers.iter().find(|c| &*c.name == n);
    if let Some(name) = name {
        return find(name);
    }
    if let Some(found) = pod
        .meta
        .annotations
        .get(DEFAULT_CONTAINER)
        .and_then(|n| find(n))
    {
        return Some(found);
    }
    let mut regular = containers
        .iter()
        .filter(|c| c.kind == ContainerKind::Regular);
    match (regular.next(), regular.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// Whether a container of `kind` that exited with `exit_code` runs again, by the pod's
/// `spec.restartPolicy` (`Always` when unset). A sidecar always restarts; an init container is
/// retried on failure unless the pod never restarts; an ephemeral container never restarts.
fn will_restart(pod: &Resource, kind: ContainerKind, exit_code: i32) -> bool {
    let policy = pod.get_str("/spec/restartPolicy").unwrap_or("Always");
    match kind {
        ContainerKind::Sidecar => true,
        ContainerKind::Ephemeral => false,
        ContainerKind::Init => exit_code != 0 && policy != "Never",
        ContainerKind::Regular => match policy {
            "Never" => false,
            "OnFailure" => exit_code != 0,
            _ => true,
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    /// A running pod with `policy`, a container `init` in state `init`, and regular containers `app`
    /// and `side` in states `app` and `side`.
    fn pod(policy: &str, init: Value, app: Value, side: Value) -> Resource {
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "web-0", "namespace": "default"},
            "spec": {
                "restartPolicy": policy,
                "initContainers": [{"name": "init"}],
                "containers": [{"name": "app"}, {"name": "side"}]
            },
            "status": {
                "phase": "Running",
                "initContainerStatuses": [{"name": "init", "state": init}],
                "containerStatuses": [
                    {"name": "app", "state": app},
                    {"name": "side", "state": side}
                ]
            }
        }))
        .unwrap()
    }

    fn exited(code: i32) -> Value {
        json!({"terminated": {"exitCode": code}})
    }

    fn running() -> Value {
        json!({"running": {}})
    }

    #[test]
    fn a_completed_init_container_in_a_running_pod_is_finished() {
        let pod = pod("Always", exited(0), running(), running());
        assert_eq!(container_fate(&pod, Some("init")), ContainerFate::Finished);
        assert_eq!(container_fate(&pod, Some("app")), ContainerFate::Running);
    }

    #[test]
    fn a_container_that_exited_next_to_a_live_one_finishes_by_the_restart_policy() {
        let done = pod("Never", exited(0), exited(0), running());
        assert_eq!(container_fate(&done, Some("app")), ContainerFate::Finished);
        let ok = pod("OnFailure", exited(0), exited(0), running());
        assert_eq!(container_fate(&ok, Some("app")), ContainerFate::Finished);
        let crashed = pod("OnFailure", exited(0), exited(2), running());
        assert_eq!(
            container_fate(&crashed, Some("app")),
            ContainerFate::Restarting
        );
        let always = pod("Always", exited(0), exited(0), running());
        assert_eq!(
            container_fate(&always, Some("app")),
            ContainerFate::Restarting
        );
    }

    #[test]
    fn a_crash_looping_container_is_restarting() {
        let waiting = json!({"waiting": {"reason": "CrashLoopBackOff"}});
        let pod = pod("Always", exited(0), waiting, running());
        assert_eq!(container_fate(&pod, Some("app")), ContainerFate::Restarting);
    }

    #[test]
    fn an_unnamed_read_follows_the_default_container() {
        let mut pod = pod("Never", exited(0), exited(0), running());
        assert_eq!(
            container_fate(&pod, None),
            ContainerFate::Running,
            "two regular containers and no annotation: cannot tell"
        );
        pod.meta
            .annotations
            .insert(DEFAULT_CONTAINER.into(), "app".into());
        assert_eq!(container_fate(&pod, None), ContainerFate::Finished);
        assert_eq!(
            container_fate(&pod, Some("missing")),
            ContainerFate::Running
        );
    }
}
