//! The container selector's list: every container of the pod spec (init and sidecar containers,
//! regular containers, ephemeral debug containers), read once from the pod.

use std::sync::Arc;

use gpui::Context;
use oxikube_domain::Resource;
use oxikube_domain::view::{ContainerKind, ContainerState, ContainerSummary, TerminatedState};
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
    /// Whether it is crash-looping: waiting in `CrashLoopBackOff`, or it restarted and its last
    /// run exited with an error. The log of its last crash is the previous instance's.
    pub crash_looping: bool,
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
        .map(|c| {
            let crashed = |t: &TerminatedState| t.exit_code != 0 || t.signal != 0;
            let crash_looping = match &c.state {
                ContainerState::Waiting { reason, .. } => {
                    reason.as_deref() == Some("CrashLoopBackOff")
                }
                ContainerState::Terminated(last) => c.restarts > 0 && crashed(last),
                _ => false,
            };
            ContainerChoice {
                name: c.name,
                kind: c.kind,
                restarted: c.restarts > 0,
                crash_looping,
            }
        })
        .collect()
}

/// Longest container name the toolbar button spells out; a longer one is cut in the middle (the
/// menu keeps the whole name).
const LABEL_NAME_CHARS: usize = 24;

/// What the toolbar's container picker says: the container read, with its place among the pod's
/// containers when there are several (`main (1/2)`).
pub(super) fn container_label(current: Option<&str>, choices: &[ContainerChoice]) -> String {
    let name = current.map_or_else(|| "default container".to_owned(), shorten);
    if choices.len() < 2 {
        return name;
    }
    match current.and_then(|c| choices.iter().position(|choice| &*choice.name == c)) {
        Some(index) => format!("{name} ({}/{})", index + 1, choices.len()),
        None => format!("{name} ({})", choices.len()),
    }
}

/// `name`, or its first and last characters around an ellipsis when it is long.
fn shorten(name: &str) -> String {
    let count = name.chars().count();
    if count <= LABEL_NAME_CHARS {
        return name.to_owned();
    }
    let keep = (LABEL_NAME_CHARS - 1) / 2;
    let head: String = name.chars().take(keep + 1).collect();
    let tail: String = name.chars().skip(count - keep).collect();
    format!("{head}\u{2026}{tail}")
}

/// The container a view reads when none was asked for: the pod's `kubectl.kubernetes.io/
/// default-container` annotation when it names one, else the first regular container.
pub fn default_container(pod: &Resource, choices: &[ContainerChoice]) -> Option<Arc<str>> {
    let annotated = pod
        .get("/metadata/annotations/kubectl.kubernetes.io~1default-container")
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
    /// Whether the container read is crash-looping and its current instance is shown: the strip
    /// under the toolbar then says that "Previous" holds the last crash.
    pub(crate) fn shows_crash_hint(&self) -> bool {
        self.aggregate.is_none()
            && !self.options.previous
            && self.options.container.as_deref().is_some_and(|name| {
                self.containers
                    .iter()
                    .any(|c| &*c.name == name && c.crash_looping)
            })
    }

    /// The containers the selector lists (empty until the pod was read).
    pub fn containers(&self) -> &[ContainerChoice] {
        &self.containers
    }

    /// Sets the pod the view reads: the selector lists its containers, and a view that names no
    /// container yet names the default one. A view that waited for the pod to know which
    /// container to read opens its stream now.
    pub fn set_pod(&mut self, pod: &Resource, cx: &mut Context<Self>) {
        self.awaiting_pod = false;
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
    /// `kubectl.kubernetes.io/default-container` annotation. Until the pod arrives, a change of
    /// what is read (range, previous instance, container) only updates the options, and the
    /// stream opens once with them. When the pod cannot be read the stream opens anyway, on the
    /// server's default.
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
        self.awaiting_pod = !self.started;
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
                None => {
                    view.awaiting_pod = false;
                    view.start_stream(cx);
                }
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
        plain.edit_json(|json| json["metadata"]["annotations"] = json!({}));
        assert_eq!(
            default_container(&plain, &choices).as_deref(),
            Some("sidecar")
        );
    }

    fn choice(name: &str) -> ContainerChoice {
        ContainerChoice {
            name: name.into(),
            kind: ContainerKind::Regular,
            restarted: false,
            crash_looping: false,
        }
    }

    #[test]
    fn the_label_counts_the_containers_when_there_are_several() {
        let two = [choice("main"), choice("sidecar")];
        assert_eq!(container_label(Some("main"), &two), "main (1/2)");
        assert_eq!(container_label(Some("sidecar"), &two), "sidecar (2/2)");
        assert_eq!(container_label(Some("gone"), &two), "gone (2)");
        assert_eq!(container_label(Some("main"), &two[..1]), "main");
        assert_eq!(container_label(None, &[]), "default container");
    }

    #[test]
    fn a_long_name_is_cut_in_the_middle() {
        let name = "a-very-long-container-name-for-a-sidecar";
        let label = container_label(Some(name), &[]);
        assert!(label.chars().count() <= LABEL_NAME_CHARS, "{label}");
        assert!(label.starts_with("a-very-long") && label.ends_with("sidecar"));
        assert!(label.contains('\u{2026}'));
    }

    #[test]
    fn a_crash_looping_container_is_flagged() {
        let pod = Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "p", "namespace": "n"},
            "spec": {"containers": [{"name": "boom"}, {"name": "ok"}, {"name": "done"}]},
            "status": {"containerStatuses": [
                {"name": "boom", "restartCount": 5,
                 "state": {"waiting": {"reason": "CrashLoopBackOff"}},
                 "lastState": {"terminated": {"exitCode": 1, "reason": "Error"}}},
                {"name": "ok", "restartCount": 1, "state": {"running": {}}},
                {"name": "done", "restartCount": 0,
                 "state": {"terminated": {"exitCode": 0, "reason": "Completed"}}}
            ]}
        }))
        .unwrap();
        let flags: Vec<bool> = choices_of(&pod).iter().map(|c| c.crash_looping).collect();
        assert_eq!(flags, [true, false, false]);
    }
}
