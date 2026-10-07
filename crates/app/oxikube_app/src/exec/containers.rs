//! Which containers of a pod a shell or an attach can open, and which one it opens.
//!
//! Pure functions over the pod object: [`PodContainers`] lists the candidates and the default
//! (`kubectl.kubernetes.io/default-container`, else the first regular container), and
//! [`plan_container`] turns a request into either one container to open or the choices a picker
//! shows with one preselected.

use std::sync::Arc;

use oxikube_domain::view::{ContainerKind, ContainerState, ContainerSummary};
use oxikube_domain::{OxiError, OxiResult, Resource};

/// `metadata.annotations` key naming the container `kubectl` opens when none is given.
pub const DEFAULT_CONTAINER_ANNOTATION: &str = "kubectl.kubernetes.io/default-container";

/// One container a session can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecContainer {
    /// The container's name.
    pub name: Arc<str>,
    /// Which list of the pod spec it is in.
    pub kind: ContainerKind,
    /// Whether it runs now (a session into a container that does not run fails).
    pub running: bool,
}

impl ExecContainer {
    /// What a picker shows: the name, with `init`, `sidecar` or `ephemeral` after it, and
    /// `not running` for a container that is not.
    pub fn label(&self) -> String {
        let mut notes = Vec::new();
        match self.kind {
            ContainerKind::Regular => {}
            ContainerKind::Init => notes.push("init"),
            ContainerKind::Sidecar => notes.push("sidecar"),
            ContainerKind::Ephemeral => notes.push("ephemeral"),
        }
        if !self.running {
            notes.push("not running");
        }
        if notes.is_empty() {
            self.name.to_string()
        } else {
            format!("{} ({})", self.name, notes.join(", "))
        }
    }
}

/// The containers of one pod a shell or attach can open, and what else the pod says about how.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PodContainers {
    containers: Vec<ExecContainer>,
    annotated: Option<Arc<str>>,
    windows: bool,
}

impl PodContainers {
    /// The candidates of `pod`: regular containers, sidecars and ephemeral containers, plus an
    /// init container while it runs. Empty for an object that is not a pod.
    pub fn of(pod: &Resource) -> Self {
        let containers = ContainerSummary::list_from_resource(pod)
            .unwrap_or_default()
            .into_iter()
            .map(|c| ExecContainer {
                running: matches!(c.state, ContainerState::Running { .. }),
                name: c.name,
                kind: c.kind,
            })
            .filter(|c| c.kind != ContainerKind::Init || c.running)
            .collect();
        let annotated = pod
            .meta
            .annotations
            .get(DEFAULT_CONTAINER_ANNOTATION)
            .map(|name| Arc::from(&**name));
        let windows = pod.get_str("/spec/os/name") == Some("windows")
            || pod.get_str("/spec/nodeSelector/kubernetes.io~1os") == Some("windows");
        Self {
            containers,
            annotated,
            windows,
        }
    }

    /// The candidates, in the order of [`ContainerSummary::list_from_resource`] (sidecars,
    /// regular, ephemeral).
    pub fn containers(&self) -> &[ExecContainer] {
        &self.containers
    }

    /// Whether the pod asks for Windows (`spec.os.name` or the `kubernetes.io/os` node selector):
    /// `bash` and `sh` do not exist there.
    pub fn is_windows(&self) -> bool {
        self.windows
    }

    /// Whether `name` is a candidate.
    pub fn contains(&self, name: &str) -> bool {
        self.containers.iter().any(|c| &*c.name == name)
    }

    /// The container opened when none is asked for: the `default-container` annotation when it
    /// names a candidate, else the first regular container, else the first candidate.
    pub fn default_container(&self) -> Option<&ExecContainer> {
        self.annotated
            .as_deref()
            .and_then(|name| self.containers.iter().find(|c| &*c.name == name))
            .or_else(|| {
                self.containers
                    .iter()
                    .find(|c| c.kind == ContainerKind::Regular)
            })
            .or_else(|| self.containers.first())
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.containers.iter().position(|c| &*c.name == name)
    }
}

/// What the picker shows for a pod with several candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerChoices {
    /// The candidates to pick from (at least two).
    pub containers: Vec<ExecContainer>,
    /// The index of the one that starts selected: the container opened last in this session,
    /// else the pod's default.
    pub preselected: usize,
}

impl ContainerChoices {
    /// The preselected container.
    pub fn preselected(&self) -> &ExecContainer {
        &self.containers[self.preselected]
    }
}

/// Which container a request opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerPlan {
    /// Open this one: it was asked for, or it is the pod's only candidate.
    Open(Arc<str>),
    /// Several candidates and none asked for: let the user pick.
    Pick(ContainerChoices),
}

/// Decides which container of `pod` a request opens.
///
/// `requested` (a container the caller named) opens as it is. Without one, a single candidate
/// opens directly and several make a [`ContainerPlan::Pick`], preselecting `remembered` (the
/// container opened last for this pod in this session) when it is still a candidate, else the
/// pod's default.
///
/// # Errors
///
/// `NotFound` for a named container the pod does not have as a candidate, `Validation` for a pod
/// with no container to open.
pub fn plan_container(
    pod: &PodContainers,
    requested: Option<&str>,
    remembered: Option<&str>,
) -> OxiResult<ContainerPlan> {
    if let Some(name) = requested {
        return match pod.containers.iter().find(|c| &*c.name == name) {
            Some(found) => Ok(ContainerPlan::Open(found.name.clone())),
            None => Err(OxiError::not_found(format!(
                "container {name} not found in the pod (or it is an init container that is not running)"
            ))),
        };
    }
    match pod.containers.as_slice() {
        [] => Err(OxiError::validation("the pod has no container to open")),
        [only] => Ok(ContainerPlan::Open(only.name.clone())),
        _ => {
            let preselected = remembered
                .and_then(|name| pod.position(name))
                .or_else(|| {
                    pod.default_container()
                        .and_then(|default| pod.position(&default.name))
                })
                .unwrap_or(0);
            Ok(ContainerPlan::Pick(ContainerChoices {
                containers: pod.containers.clone(),
                preselected,
            }))
        }
    }
}

/// The container a request without a pick opens (a palette or agent command that names none):
/// `requested`, else the pod's default.
///
/// # Errors
///
/// As [`plan_container`].
pub fn container_to_open(pod: &PodContainers, requested: Option<&str>) -> OxiResult<Arc<str>> {
    match plan_container(pod, requested, None)? {
        ContainerPlan::Open(name) => Ok(name),
        ContainerPlan::Pick(_) => pod
            .default_container()
            .map(|c| c.name.clone())
            .ok_or_else(|| OxiError::validation("the pod has no container to open")),
    }
}
