//! [`PodSummary`] and [`ContainerSummary`].
//!
//! [`PodSummary::status`] is the `STATUS` column of `kubectl get pods`. The rules, in the order
//! the Kubernetes pod printer applies them:
//!
//! 1. Start from `status.reason` if set, else `status.phase`. A `PodScheduled` condition with
//!    reason `SchedulingGated` overrides both.
//! 2. Walk `status.initContainerStatuses` in order. Containers that exited 0, and started
//!    restartable init containers (sidecars, `restartPolicy: Always`), are skipped. The first
//!    other container decides the status and stops the walk: `Init:<reason>` (or
//!    `Init:Signal:N` / `Init:ExitCode:N` when the reason is empty) if it terminated,
//!    `Init:<reason>` if it is waiting with a reason other than `PodInitializing`, otherwise
//!    `Init:<index>/<count>`.
//! 3. If no init container is pending, or the `Initialized` condition is `True`, walk
//!    `status.containerStatuses` from last to first so the first container wins: a waiting
//!    reason, a terminated reason (or `Signal:N` / `ExitCode:N`), and running ready containers
//!    count towards `ready`. A `Completed` result becomes `Running` if a container still runs
//!    and the pod is `Ready`, the reason of a container that exited non-zero if there is one,
//!    or `NotReady` if a container still runs.
//! 4. With `deletionTimestamp` set, `status.reason == "NodeLost"` shows `Unknown`; otherwise a
//!    pod that is not `Succeeded` or `Failed` shows `Terminating`.
//!
//! The restart count follows the printer too: while init containers are still running it sums
//! every init container; afterwards it sums sidecars and regular containers.

use std::borrow::Cow;
use std::sync::Arc;

use jiff::Timestamp;
use serde_json::Value;

use super::{
    ViewError, arc_of, arr_of, bool_of, check_kind, count_of, i32_of, obj_of, str_of, sub, ts_of,
};
use crate::age::Age;
use crate::resource::Resource;

/// `status.phase` of a Pod.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PodPhase {
    /// Accepted, but at least one container has not started.
    Pending,
    /// Bound to a node with at least one container running or starting.
    Running,
    /// Every container terminated successfully and will not restart.
    Succeeded,
    /// Every container terminated and at least one failed.
    Failed,
    /// The phase is `Unknown`, missing, or not one the client recognises.
    #[default]
    Unknown,
}

impl PodPhase {
    /// Parse `status.phase`; anything unrecognised is [`PodPhase::Unknown`].
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("Pending") => Self::Pending,
            Some("Running") => Self::Running,
            Some("Succeeded") => Self::Succeeded,
            Some("Failed") => Self::Failed,
            _ => Self::Unknown,
        }
    }

    /// The API spelling of the phase.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Running => "Running",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Unknown => "Unknown",
        }
    }

    /// Whether the phase is final (`Succeeded` or `Failed`).
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// `status.qosClass` of a Pod.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QosClass {
    /// Every container has equal CPU and memory requests and limits.
    Guaranteed,
    /// At least one request or limit, but not `Guaranteed`.
    Burstable,
    /// No requests or limits.
    BestEffort,
}

impl QosClass {
    /// Parse `status.qosClass`; `None` when absent or unrecognised.
    pub fn parse(s: Option<&str>) -> Option<Self> {
        match s? {
            "Guaranteed" => Some(Self::Guaranteed),
            "Burstable" => Some(Self::Burstable),
            "BestEffort" => Some(Self::BestEffort),
            _ => None,
        }
    }

    /// The API spelling of the class.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Guaranteed => "Guaranteed",
            Self::Burstable => "Burstable",
            Self::BestEffort => "BestEffort",
        }
    }
}

/// One row of a pod table: the `kubectl get pods -o wide` columns, typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `metadata.namespace`.
    pub namespace: Option<Arc<str>>,
    /// `status.phase`.
    pub phase: PodPhase,
    /// The `STATUS` column, per the kubectl pod printer (see the [module docs](self)). A pod
    /// with neither `status.phase` nor `status.reason` shows `Unknown`.
    pub status: Arc<str>,
    /// Ready containers (regular containers plus started sidecars), the left of `READY`.
    pub ready: u32,
    /// Regular containers plus sidecars in the spec, the right of `READY`.
    pub total: u32,
    /// The `RESTARTS` count, per the kubectl pod printer.
    pub restarts: u32,
    /// Latest `lastState.terminated.finishedAt` among the containers counted in `restarts`.
    pub last_restart: Option<Timestamp>,
    /// `status.qosClass`.
    pub qos: Option<QosClass>,
    /// First of `status.podIPs`, else `status.podIP`.
    pub ip: Option<Arc<str>>,
    /// `spec.nodeName`.
    pub node: Option<Arc<str>>,
    /// `status.nominatedNodeName`.
    pub nominated_node: Option<Arc<str>>,
    /// `metadata.creationTimestamp`.
    pub created: Option<Timestamp>,
}

impl PodSummary {
    /// Build the summary of a Pod.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a core `Pod`. Missing or malformed fields never
    /// fail; they fall back to defaults.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "Pod", &[("", "Pod")])?;
        let spec = sub(&res.json, "spec");
        let status = sub(&res.json, "status");
        let row = StatusRow::compute(spec, status, res.meta.deletion.is_some());
        let ip = arr_of(status, "podIPs")
            .first()
            .and_then(|p| arc_of(p, "ip"))
            .or_else(|| arc_of(status, "podIP"));
        Ok(Self {
            name: res.meta.name.clone(),
            namespace: res.meta.namespace.clone(),
            phase: PodPhase::parse(str_of(status, "phase")),
            status: Arc::from(&*row.reason),
            ready: row.ready,
            total: row.total,
            restarts: row.restarts,
            last_restart: row.last_restart,
            qos: QosClass::parse(str_of(status, "qosClass")),
            ip,
            node: arc_of(spec, "nodeName"),
            nominated_node: arc_of(status, "nominatedNodeName"),
            created: res.meta.creation,
        })
    }

    /// The `READY` column, `ready/total`.
    pub fn ready_display(&self) -> String {
        format!("{}/{}", self.ready, self.total)
    }

    /// The `RESTARTS` column: `3 (5m ago)` when a last restart time is known, else `3`.
    pub fn restarts_display(&self, now: Timestamp) -> String {
        match self.last_restart {
            Some(at) if self.restarts != 0 => {
                format!("{} ({} ago)", self.restarts, Age::between(at, now))
            }
            _ => self.restarts.to_string(),
        }
    }

    /// Age at `now`, if the creation time is known.
    pub fn age(&self, now: Timestamp) -> Option<Age> {
        self.created.map(|c| Age::between(c, now))
    }
}

/// The printer's derived columns, before conversion to owned output.
struct StatusRow<'a> {
    reason: Cow<'a, str>,
    ready: u32,
    total: u32,
    restarts: u32,
    last_restart: Option<Timestamp>,
}

impl<'a> StatusRow<'a> {
    fn compute(spec: &'a Value, status: &'a Value, deleting: bool) -> Self {
        let phase = str_of(status, "phase");
        let status_reason = str_of(status, "reason");
        let mut reason = Cow::Borrowed(status_reason.or(phase).unwrap_or("Unknown"));

        let conditions = arr_of(status, "conditions");
        if conditions.iter().any(|c| {
            str_of(c, "type") == Some("PodScheduled")
                && str_of(c, "reason") == Some("SchedulingGated")
        }) {
            reason = Cow::Borrowed("SchedulingGated");
        }

        let init_specs = arr_of(spec, "initContainers");
        let sidecars = init_specs.iter().filter(|c| is_restartable(c)).count();
        let total = len_u32(arr_of(spec, "containers").len() + sidecars);

        let mut ready = 0u32;
        let mut restarts = 0u32;
        let mut last_restart = None;
        let mut sidecar_restarts = 0u32;
        let mut sidecar_last_restart = None;
        let mut initializing = false;

        for (i, cs) in arr_of(status, "initContainerStatuses").iter().enumerate() {
            let count = count_of(cs, "restartCount");
            let finished = last_finished_at(cs);
            restarts = restarts.saturating_add(count);
            last_restart = later(last_restart, finished);
            let restartable = str_of(cs, "name")
                .and_then(|name| init_specs.iter().find(|c| str_of(c, "name") == Some(name)))
                .is_some_and(is_restartable);
            if restartable {
                sidecar_restarts = sidecar_restarts.saturating_add(count);
                sidecar_last_restart = later(sidecar_last_restart, finished);
            }

            let state = StateView::of(cs);
            if state.terminated.is_some_and(|t| i32_of(t, "exitCode") == 0) {
                continue;
            }
            if restartable && bool_of(cs, "started") {
                if bool_of(cs, "ready") {
                    ready += 1;
                }
                continue;
            }
            initializing = true;
            let waiting_reason = state
                .waiting
                .and_then(|w| str_of(w, "reason"))
                .filter(|r| *r != "PodInitializing");
            reason = Cow::Owned(match (state.terminated, waiting_reason) {
                (Some(t), _) => match str_of(t, "reason") {
                    Some(r) => format!("Init:{r}"),
                    None => exit_text("Init:", t),
                },
                (None, Some(r)) => format!("Init:{r}"),
                (None, None) => format!("Init:{i}/{}", init_specs.len()),
            });
            break;
        }

        if !initializing || initialized(conditions) {
            restarts = sidecar_restarts;
            last_restart = sidecar_last_restart;
            let mut has_running = false;
            let mut error_reason = None;
            for cs in arr_of(status, "containerStatuses").iter().rev() {
                restarts = restarts.saturating_add(count_of(cs, "restartCount"));
                last_restart = later(last_restart, last_finished_at(cs));
                let state = StateView::of(cs);
                if let Some(r) = state.waiting.and_then(|w| str_of(w, "reason")) {
                    reason = Cow::Borrowed(r);
                } else if let Some(t) = state.terminated {
                    reason = match str_of(t, "reason") {
                        Some(r) => Cow::Borrowed(r),
                        None => Cow::Owned(exit_text("", t)),
                    };
                    if i32_of(t, "exitCode") != 0 {
                        error_reason = Some(reason.clone());
                    }
                } else if bool_of(cs, "ready") && state.running.is_some() {
                    has_running = true;
                    ready += 1;
                }
            }
            if reason == "Completed" {
                let pod_ready = conditions.iter().any(|c| {
                    str_of(c, "type") == Some("Ready") && str_of(c, "status") == Some("True")
                });
                if has_running && pod_ready {
                    reason = Cow::Borrowed("Running");
                } else if let Some(err) = error_reason {
                    reason = err;
                } else if has_running {
                    reason = Cow::Borrowed("NotReady");
                }
            }
        }

        if deleting && status_reason == Some("NodeLost") {
            reason = Cow::Borrowed("Unknown");
        } else if deleting && !PodPhase::parse(phase).is_terminal() {
            reason = Cow::Borrowed("Terminating");
        }

        Self {
            reason,
            ready,
            total,
            restarts,
            last_restart,
        }
    }
}

/// The first `Initialized` condition is `True`.
fn initialized(conditions: &[Value]) -> bool {
    conditions
        .iter()
        .find(|c| str_of(c, "type") == Some("Initialized"))
        .is_some_and(|c| str_of(c, "status") == Some("True"))
}

/// An init container spec with `restartPolicy: Always` (a sidecar).
fn is_restartable(spec: &Value) -> bool {
    str_of(spec, "restartPolicy") == Some("Always")
}

/// `Signal:N` or `ExitCode:N` (with `prefix`) for a terminated state with no reason.
fn exit_text(prefix: &str, terminated: &Value) -> String {
    match i32_of(terminated, "signal") {
        0 => format!("{prefix}ExitCode:{}", i32_of(terminated, "exitCode")),
        sig => format!("{prefix}Signal:{sig}"),
    }
}

/// `lastState.terminated.finishedAt` of a container status.
fn last_finished_at(cs: &Value) -> Option<Timestamp> {
    obj_of(sub(cs, "lastState"), "terminated").and_then(|t| ts_of(t, "finishedAt"))
}

fn later(a: Option<Timestamp>, b: Option<Timestamp>) -> Option<Timestamp> {
    a.max(b)
}

fn len_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Borrowed view of a container status's `state` (each key present only when set).
struct StateView<'a> {
    waiting: Option<&'a Value>,
    running: Option<&'a Value>,
    terminated: Option<&'a Value>,
}

impl<'a> StateView<'a> {
    fn of(container_status: &'a Value) -> Self {
        let state = sub(container_status, "state");
        Self {
            waiting: obj_of(state, "waiting"),
            running: obj_of(state, "running"),
            terminated: obj_of(state, "terminated"),
        }
    }
}

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
    fn from_json(v: &Value) -> Self {
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
        let spec = sub(&res.json, "spec");
        let status = sub(&res.json, "status");
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
                let cs = statuses.iter().find(|s| str_of(s, "name") == Some(name));
                out.push(Self::build(name, c, cs, kind));
            }
        }
        Ok(out)
    }

    fn build(name: &str, spec: &Value, status: Option<&Value>, kind: ContainerKind) -> Self {
        let image = arc_of(spec, "image").or_else(|| status.and_then(|s| arc_of(s, "image")));
        let Some(cs) = status else {
            return Self {
                name: Arc::from(name),
                image,
                kind,
                ready: false,
                started: false,
                restarts: 0,
                state: ContainerState::Unknown,
                last_termination: None,
            };
        };
        Self {
            name: Arc::from(name),
            image,
            kind,
            ready: bool_of(cs, "ready"),
            started: bool_of(cs, "started"),
            restarts: count_of(cs, "restartCount"),
            state: ContainerState::from_view(&StateView::of(cs)),
            last_termination: obj_of(sub(cs, "lastState"), "terminated")
                .map(TerminatedState::from_json),
        }
    }
}
