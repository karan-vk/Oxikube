//! The container selector's list: every container of the pod spec (init and sidecar containers,
//! regular containers, ephemeral debug containers), read once from the pod.

use std::sync::Arc;

use gpui::Context;
use oxikube_domain::Resource;
use oxikube_domain::view::{ContainerKind, ContainerSummary};
use oxikube_runtime::{notify_coalesced, spawn_kube};
use oxikube_workspace::ItemEvent;

use super::LogView;

/// One entry of the container selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerChoice {
    /// The container's name.
    pub name: Arc<str>,
    /// Which list of the pod spec it is in.
    pub kind: ContainerKind,
    /// Whether it has restarted (so a previous instance may have a log).
    pub restarted: bool,
}

impl ContainerChoice {
    /// What the selector shows: the name, with `init`, `sidecar` or `ephemeral` after it.
    pub fn label(&self) -> String {
        match self.kind {
            ContainerKind::Regular => self.name.to_string(),
            ContainerKind::Init => format!("{} (init)", self.name),
            ContainerKind::Sidecar => format!("{} (sidecar)", self.name),
            ContainerKind::Ephemeral => format!("{} (ephemeral)", self.name),
        }
    }
}

/// The choices of `pod`, in spec order (init and sidecars, regular, ephemeral); empty for an
/// object that is not a pod.
pub fn choices_of(pod: &Resource) -> Vec<ContainerChoice> {
    ContainerSummary::list_from_resource(pod)
        .unwrap_or_default()
        .into_iter()
        .map(|c| ContainerChoice {
            name: c.name,
            kind: c.kind,
            restarted: c.restarts > 0,
        })
        .collect()
}

/// The container a view reads when none was asked for: the pod's `kubectl.kubernetes.io/
/// default-container` annotation when it names one, else the first regular container.
pub fn default_container(pod: &Resource, choices: &[ContainerChoice]) -> Option<Arc<str>> {
    let annotated = pod
        .json
        .pointer("/metadata/annotations/kubectl.kubernetes.io~1default-container")
        .and_then(|value| value.as_str())
        .filter(|name| choices.iter().any(|c| &*c.name == *name));
    annotated.map(Arc::from).or_else(|| {
        choices
            .iter()
            .find(|c| c.kind == ContainerKind::Regular)
            .map(|c| c.name.clone())
    })
}

impl LogView {
    /// The containers the selector lists (empty until the pod was read).
    pub fn containers(&self) -> &[ContainerChoice] {
        &self.containers
    }

    /// Sets the pod the view reads: the selector lists its containers, and a view that names no
    /// container yet names the default one. A view that waited for the pod to know which
    /// container to read opens its stream now.
    pub fn set_pod(&mut self, pod: &Resource, cx: &mut Context<Self>) {
        self.containers = choices_of(pod);
        if self.options.container.is_none()
            && let Some(name) = default_container(pod, &self.containers)
        {
            self.options.container = Some(name.to_string());
            cx.emit(ItemEvent::UpdateTab);
        }
        self.start_stream(cx);
        notify_coalesced(cx);
    }

    /// Reads the pod once (on the Tokio bridge) for the container selector. A view that names no
    /// container waits for it before it opens its stream: the API server refuses a log read
    /// without a container for a pod that has several, and only the client knows the
    /// `kubectl.kubernetes.io/default-container` annotation. When the pod cannot be read the
    /// stream opens anyway, on the server's default.
    pub(super) fn load_pod(&mut self, cx: &mut Context<Self>) {
        let Some(reader) = self
            .deps
            .sessions
            .get(&self.target.cluster)
            .and_then(|session| session.resources())
        else {
            self.start_stream(cx);
            return;
        };
        let target = self.target.clone();
        let read = spawn_kube(cx, async move {
            reader
                .get(&target.gvk, target.namespace.as_deref(), &target.name)
                .await
        });
        self.pod_task = Some(cx.spawn(async move |this, cx| {
            let pod = match read.await {
                Ok(Ok(pod)) => Some(pod),
                _ => None,
            };
            this.update(cx, |view, cx| match pod {
                Some(pod) => view.set_pod(&pod, cx),
                None => view.start_stream(cx),
            })
            .ok();
        }));
    }

    /// Opens the stream unless it was opened already.
    fn start_stream(&mut self, cx: &mut Context<Self>) {
        if !self.started {
            self.open_stream(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pod() -> Resource {
        Resource::from_json(json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {"name": "web-0", "namespace": "shop",
                "annotations": {"kubectl.kubernetes.io/default-container": "app"}},
            "spec": {
                "initContainers": [{"name": "migrate"}, {"name": "proxy", "restartPolicy": "Always"}],
                "containers": [{"name": "sidecar"}, {"name": "app"}],
                "ephemeralContainers": [{"name": "debugger"}]
            },
            "status": {"containerStatuses": [{"name": "app", "restartCount": 2}]}
        }))
        .unwrap()
    }

    #[test]
    fn lists_init_regular_and_ephemeral_containers_in_spec_order() {
        let choices = choices_of(&pod());
        let labels: Vec<String> = choices.iter().map(ContainerChoice::label).collect();
        assert_eq!(
            labels,
            [
                "migrate (init)",
                "proxy (sidecar)",
                "sidecar",
                "app",
                "debugger (ephemeral)"
            ]
        );
        assert!(choices[3].restarted && !choices[2].restarted);
    }

    #[test]
    fn the_default_is_the_annotated_container_else_the_first_regular_one() {
        let pod = pod();
        let choices = choices_of(&pod);
        assert_eq!(default_container(&pod, &choices).as_deref(), Some("app"));
        let mut plain = pod;
        plain.json["metadata"]["annotations"] = json!({});
        assert_eq!(
            default_container(&plain, &choices).as_deref(),
            Some("sidecar")
        );
    }
}
