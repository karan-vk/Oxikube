//! The pods of an aggregate as the watch describes them, and which of their containers can be
//! streamed now.

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::view::{ContainerKind, ContainerState, ContainerSummary};

/// One container of a pod, as far as reading its log goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PodContainer {
    pub name: Arc<str>,
    /// Whether a log read can succeed now: the container runs or ran (or restarted, so a previous
    /// instance exists). A container still waiting to start has no log yet; it is read once an
    /// update of the pod says it started.
    pub streamable: bool,
}

/// A pod the selector matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PodState {
    pub name: Arc<str>,
    /// `metadata.uid` (the name when the object has none): a pod deleted and recreated under the
    /// same name (a StatefulSet's) is a new pod with new streams.
    pub uid: Arc<str>,
    pub containers: Vec<PodContainer>,
    /// Whether the pod appeared after the view opened (not in the first list): it is new, so its
    /// log is read from the start (E08-S07).
    pub joined: bool,
}

impl PodState {
    /// The pod `resource` describes, with the containers to read: regular containers and sidecars
    /// (init containers and ephemeral debug containers are the single-pod viewer's), narrowed to
    /// `only` when a container name is asked for. With `previous` only containers that restarted
    /// have a log to read.
    pub fn of(resource: &Resource, only: Option<&str>, previous: bool) -> Self {
        let containers = ContainerSummary::list_from_resource(resource)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| matches!(c.kind, ContainerKind::Regular | ContainerKind::Sidecar))
            .filter(|c| only.is_none_or(|name| &*c.name == name))
            .map(|c| {
                let ran = matches!(
                    c.state,
                    ContainerState::Running { .. } | ContainerState::Terminated(_)
                );
                let streamable = if previous {
                    c.restarts > 0
                } else {
                    ran || c.restarts > 0
                };
                PodContainer {
                    name: c.name,
                    streamable,
                }
            })
            .collect();
        Self {
            name: Arc::from(resource.name()),
            uid: resource
                .meta
                .uid
                .clone()
                .unwrap_or_else(|| Arc::from(resource.name())),
            containers,
            joined: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pod(statuses: serde_json::Value) -> Resource {
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "web-7d9", "namespace": "shop"},
            "spec": {
                "initContainers": [{"name": "migrate"}, {"name": "proxy", "restartPolicy": "Always"}],
                "containers": [{"name": "app"}, {"name": "metrics"}],
                "ephemeralContainers": [{"name": "debugger"}]
            },
            "status": {"containerStatuses": statuses, "initContainerStatuses": [
                {"name": "proxy", "state": {"running": {}}}
            ]}
        }))
        .unwrap()
    }

    fn names(pod: &PodState) -> Vec<(&str, bool)> {
        pod.containers
            .iter()
            .map(|c| (&*c.name, c.streamable))
            .collect()
    }

    #[test]
    fn regular_containers_and_sidecars_are_read_and_init_and_ephemeral_are_not() {
        let state = PodState::of(
            &pod(json!([
                {"name": "app", "state": {"running": {}}},
                {"name": "metrics", "state": {"waiting": {"reason": "ContainerCreating"}}}
            ])),
            None,
            false,
        );
        assert_eq!(
            names(&state),
            [("proxy", true), ("app", true), ("metrics", false)],
            "a waiting container is read once it starts"
        );
    }

    #[test]
    fn a_crash_looping_container_has_a_log_to_read() {
        let state = PodState::of(
            &pod(json!([
                {"name": "app", "restartCount": 3, "state": {"waiting": {"reason": "CrashLoopBackOff"}}}
            ])),
            Some("app"),
            false,
        );
        assert_eq!(names(&state), [("app", true)]);
    }

    #[test]
    fn the_previous_instance_exists_only_after_a_restart() {
        let state = PodState::of(
            &pod(json!([
                {"name": "app", "restartCount": 1, "state": {"running": {}}},
                {"name": "metrics", "state": {"running": {}}}
            ])),
            None,
            true,
        );
        assert_eq!(
            names(&state),
            [("proxy", false), ("app", true), ("metrics", false)]
        );
    }

    #[test]
    fn a_container_name_narrows_the_pod() {
        let state = PodState::of(&pod(json!([])), Some("metrics"), false);
        assert_eq!(names(&state), [("metrics", false)]);
    }
}
