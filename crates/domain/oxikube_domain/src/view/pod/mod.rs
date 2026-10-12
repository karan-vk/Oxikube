//! [`PodSummary`] and [`ContainerSummary`]: the Pod view-models behind the pods table and the
//! containers drill-down (E12-S02).
//!
//! [`PodSummary::status`] is the `STATUS` column of `kubectl get pods`, ported fresh (no code
//! copied, and not kdash's `get_status`) from kubectl's pod printer: `printPod` in
//! `pkg/printers/internalversion/printers.go` of Kubernetes **v1.36** (`release-1.36`, the API
//! version the workspace's `k8s-openapi` pins), checked case by case against that release's
//! `TestPrintPod` and `TestPrintPodWithRestartableInitContainer` (`tests/pod_printer_v136.rs`).
//! The rules, in the order the printer applies them:
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
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | [`PodPhase`], [`QosClass`], [`PodSummary`] |
//! | `status` | the printer's `READY` / `STATUS` / `RESTARTS` derivation |
//! | `container` | [`ContainerSummary`] and its [`ContainerState`] |
//! | `detail` | [`PodDetail`] (E12-S03): environment, mounts, probes, volumes, tolerations, affinity, node and owners for the detail panel |
//! | `resources` | [`ComputeResources`] (requests and limits, the pod's effective totals), [`Probes`], [`ContainerPort`] |

mod container;
mod detail;
mod resources;
mod status;

use std::sync::Arc;

use jiff::Timestamp;

pub use container::{ContainerKind, ContainerState, ContainerSummary, TerminatedState};
pub use detail::{
    AffinityLine, ContainerDetail, EnvFrom, EnvRow, EnvSource, EnvSources, EnvValue, EnvVar, MASK,
    Missing, MountSummary, Origin, PodDetail, ProbeDetail, ProbeKind, SecretValue, SourceData,
    SourceKind, SourceState, SourceValue, Toleration, VolumeSummary, node_of,
};
pub use resources::{ComputeResources, ContainerPort, Probes, ResourceAmounts};
pub(super) use status::{StateView, is_restartable};

use super::{ViewError, arc_of, arr_of, check_kind, str_of, sub};
use crate::age::Age;
use crate::resource::{OwnerRef, Resource};
use status::StatusRow;

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
    /// The `STATUS` column, per the kubectl pod printer (rules are listed at the top of this file). A pod
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
    /// `spec.serviceAccountName` (else the deprecated `spec.serviceAccount`).
    pub service_account: Option<Arc<str>>,
    /// The controller among `metadata.ownerReferences` (the `Controlled By` column).
    pub controlled_by: Option<OwnerRef>,
    /// The pod's effective CPU and memory requests and limits ([`ComputeResources`]): what the
    /// `%CPU/R`, `%CPU/L`, `%MEM/R` and `%MEM/L` columns divide usage by.
    pub resources: ComputeResources,
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
        let spec = sub(res.json(), "spec");
        let status = sub(res.json(), "status");
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
            service_account: arc_of(spec, "serviceAccountName")
                .or_else(|| arc_of(spec, "serviceAccount")),
            controlled_by: res.meta.controller_ref().cloned(),
            resources: ComputeResources::of_pod(spec),
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
