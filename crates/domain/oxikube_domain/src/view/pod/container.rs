//! [`ContainerSummary`]: one container of a pod (init, sidecar, regular or ephemeral), its spec
//! entry joined with its status entry: the rows of the containers drill-down.

use std::sync::Arc;

use crate::json::JsonRef;
use crate::resource::Resource;
use crate::view::{
    ViewError, arc_of, arr_of, bool_of, check_kind, count_of, i32_of, obj_of, str_of, sub, ts_of,
};
use jiff::Timestamp;

use super::resources::{ComputeResources, ContainerPort, Probes};
use super::status::{StateView, is_restartable};

/// Which list of the pod spec a container comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerKind {
    /// `spec.initContainers` entry that runs to completion before the pod starts.
    Init,
    /// `spec.initContainers` entry with `restartPolicy: Always` that keeps running.
    Sidecar,
    /// `spec.containers` entry.
    Regular,
    /// `spec.ephemeralContainers` entry (debug container).
    Ephemeral,
}

impl ContainerKind {
    /// The type marker a containers table shows next to the name: `init`, `sidecar` or
    /// `ephemeral`; empty for a regular container.
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Sidecar => "sidecar",
            Self::Regular => "",
            Self::Ephemeral => "ephemeral",
        }
    }
}

/// A terminated container state (`state.terminated` or `lastState.terminated`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminatedState {
    /// `exitCode`; zero when absent.
    pub exit_code: i32,
    /// `signal`; zero when absent.
    pub signal: i32,
    /// `reason`, for example `Completed`, `Error` or `OOMKilled`.
    pub reason: Option<Arc<str>>,
    /// `message`.
    pub message: Option<Arc<str>>,
    /// `startedAt`.
    pub started_at: Option<Timestamp>,
    /// `finishedAt`.
    pub finished_at: Option<Timestamp>,
}

impl TerminatedState {
    fn from_json(v: JsonRef<'_>) -> Self {
        Self {
            exit_code: i32_of(v, "exitCode"),
            signal: i32_of(v, "signal"),
            reason: arc_of(v, "reason"),
            message: arc_of(v, "message"),
            started_at: ts_of(v, "startedAt"),
            finished_at: ts_of(v, "finishedAt"),
        }
    }
}

/// The current state of one container.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ContainerState {
    /// Not running yet, for example `ContainerCreating` or `CrashLoopBackOff`.
    Waiting {
        /// `reason`.
        reason: Option<Arc<str>>,
        /// `message`.
        message: Option<Arc<str>>,
    },
    /// Running.
    Running {
        /// `startedAt`.
        started_at: Option<Timestamp>,
    },
    /// Exited.
    Terminated(TerminatedState),
    /// No status reported yet, or none of the known states is set.
    #[default]
    Unknown,
}

impl ContainerState {
    fn from_view(view: &StateView<'_>) -> Self {
        if let Some(w) = view.waiting {
            Self::Waiting {
                reason: arc_of(w, "reason"),
                message: arc_of(w, "message"),
            }
        } else if let Some(t) = view.terminated {
            Self::Terminated(TerminatedState::from_json(t))
        } else if let Some(r) = view.running {
            Self::Running {
                started_at: ts_of(r, "startedAt"),
            }
        } else {
            Self::Unknown
        }
    }

    /// Short label for a table cell: the reason when there is one, else the state name.
    pub fn label(&self) -> &str {
        match self {
            Self::Waiting { reason, .. } => reason.as_deref().unwrap_or("Waiting"),
            Self::Running { .. } => "Running",
            Self::Terminated(t) => t.reason.as_deref().unwrap_or("Terminated"),
            Self::Unknown => "Unknown",
        }
    }
}

/// One container of a pod: its spec entry joined with its status entry by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSummary {
    /// Container name.
    pub name: Arc<str>,
    /// `image` from the spec, else the image the status reports.
    pub image: Option<Arc<str>>,
    /// Which spec list the container comes from.
    pub kind: ContainerKind,
    /// `ready` from the status.
    pub ready: bool,
    /// `started` from the status (startup probe passed); `false` when absent.
    pub started: bool,
    /// `restartCount` from the status.
    pub restarts: u32,
    /// Current state.
    pub state: ContainerState,
    /// `lastState.terminated`, the previous run's exit.
    pub last_termination: Option<TerminatedState>,
    /// CPU and memory requests and limits from the spec.
    pub resources: ComputeResources,
    /// Which probes the spec defines.
    pub probes: Probes,
    /// `ports` from the spec.
    pub ports: Arc<[ContainerPort]>,
}

impl ContainerSummary {
    /// Every container of a Pod: init containers and sidecars first, then regular containers,
    /// then ephemeral containers, each in spec order. Spec entries without a name are skipped.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a core `Pod`.
    pub fn list_from_resource(res: &Resource) -> Result<Vec<Self>, ViewError> {
        check_kind(res, "Pod", &[("", "Pod")])?;
        let spec = sub(res.json(), "spec");
        let status = sub(res.json(), "status");
        let groups = [
            (
                "initContainers",
                "initContainerStatuses",
                ContainerKind::Init,
            ),
            ("containers", "containerStatuses", ContainerKind::Regular),
            (
                "ephemeralContainers",
                "ephemeralContainerStatuses",
                ContainerKind::Ephemeral,
            ),
        ];
        let len: usize = groups.iter().map(|(s, _, _)| arr_of(spec, s).len()).sum();
        let mut out = Vec::with_capacity(len);
        for (spec_key, status_key, kind) in groups {
            let statuses = arr_of(status, status_key);
            for c in arr_of(spec, spec_key) {
                let Some(name) = str_of(c, "name") else {
                    continue;
                };
                let kind = if kind == ContainerKind::Init && is_restartable(c) {
                    ContainerKind::Sidecar
                } else {
                    kind
                };
                let cs = statuses.iter().find(|&s| str_of(s, "name") == Some(name));
                out.push(Self::build(name, c, cs, kind));
            }
        }
        Ok(out)
    }

    fn build(
        name: &str,
        spec: JsonRef<'_>,
        status: Option<JsonRef<'_>>,
        kind: ContainerKind,
    ) -> Self {
        let image = arc_of(spec, "image").or_else(|| status.and_then(|s| arc_of(s, "image")));
        let base = Self {
            name: Arc::from(name),
            image,
            kind,
            ready: false,
            started: false,
            restarts: 0,
            state: ContainerState::Unknown,
            last_termination: None,
            resources: ComputeResources::of_container(spec),
            probes: Probes::of(spec),
            ports: ContainerPort::list(spec),
        };
        let Some(cs) = status else {
            return base;
        };
        Self {
            ready: bool_of(cs, "ready"),
            started: bool_of(cs, "started"),
            restarts: count_of(cs, "restartCount"),
            state: ContainerState::from_view(&StateView::of(cs)),
            last_termination: obj_of(sub(cs, "lastState"), "terminated")
                .map(TerminatedState::from_json),
            ..base
        }
    }
}
