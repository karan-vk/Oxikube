//! A pod's effective requests and limits, and where it is allocated.

use std::sync::Arc;

use super::quantities::{amounts_of, override_with};
use super::{GpuVendors, ResourceAmounts};
use crate::json::JsonRef;
use crate::resource::Resource;
use crate::view::{PodPhase, ViewError, arc_of, arr_of, check_kind, str_of, sub};

/// What a pod asks for: the Kubernetes effective requests and limits (see the
/// [module docs](super) for the rules).
///
/// An amount of zero means "not set". In-place pod resize (`status.resize`) is not modelled: the
/// figures are those of the spec.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PodResources {
    /// Effective requests: what the scheduler books on the node.
    pub requests: ResourceAmounts,
    /// Effective limits: what the pod may use at most. A resource without a limit on some
    /// container contributes nothing for that container, as in Kubernetes.
    pub limits: ResourceAmounts,
}

impl PodResources {
    /// Whether the pod sets no request and no limit (a `BestEffort` pod).
    pub fn is_empty(&self) -> bool {
        self.requests.is_zero() && self.limits.is_zero()
    }
}

/// A pod's requests and limits, summed the way the scheduler does.
///
/// Per resource: the sum over regular containers and restartable init containers (sidecars), or
/// the peak of an init container running with the sidecars started before it, whichever is
/// larger; plus `spec.overhead`; with `spec.resources` replacing the sum for the resources it
/// sets. `gpus` names the extended resources counted as GPUs.
///
/// The story text asks for this over a `PodSummary`; that view-model carries no container specs
/// (it is built for every row of a 10 000-pod table), so this reads the pod's JSON directly.
///
/// # Errors
///
/// [`ViewError::WrongKind`] when `pod` is not a core `Pod`. Missing or malformed fields never
/// fail: they are zero.
pub fn aggregate_pod_requests_limits(
    pod: &Resource,
    gpus: &GpuVendors,
) -> Result<PodResources, ViewError> {
    check_kind(pod, "Pod", &[("", "Pod")])?;
    Ok(from_spec(sub(pod.json(), "spec"), gpus))
}

fn from_spec(spec: JsonRef<'_>, gpus: &GpuVendors) -> PodResources {
    let overhead = amounts_of(sub(spec, "overhead"), gpus);
    let mut requests = effective(spec, "requests", gpus);
    requests += &overhead;
    let mut limits = effective(spec, "limits", gpus);
    // Overhead rides on a limit only where the pod has one (`PodRequestsAndLimits`).
    if !limits.cpu.is_zero() {
        limits.cpu += overhead.cpu;
    }
    if !limits.memory.is_zero() {
        limits.memory += overhead.memory;
    }
    PodResources { requests, limits }
}

/// Containers, init containers and pod-level resources for one of `requests` / `limits`.
fn effective(spec: JsonRef<'_>, key: &str, gpus: &GpuVendors) -> ResourceAmounts {
    let of = |container: JsonRef<'_>| amounts_of(sub(sub(container, "resources"), key), gpus);

    let mut total = ResourceAmounts::zero();
    for container in arr_of(spec, "containers") {
        total += &of(container);
    }
    let mut sidecars = ResourceAmounts::zero();
    let mut init_peak = ResourceAmounts::zero();
    for container in arr_of(spec, "initContainers") {
        let mut amounts = of(container);
        if str_of(container, "restartPolicy") == Some("Always") {
            total += &amounts;
            sidecars += &amounts;
            init_peak.max_assign(&sidecars);
        } else {
            amounts += &sidecars;
            init_peak.max_assign(&amounts);
        }
    }
    total.max_assign(&init_peak);
    override_with(&mut total, sub(sub(spec, "resources"), key), gpus);
    total
}

/// A pod as the allocation sums see it: where it runs, whether it still holds resources, and
/// what it asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodAllocation {
    /// `metadata.namespace`.
    pub namespace: Option<Arc<str>>,
    /// `spec.nodeName`; `None` for a pod the scheduler has not placed yet.
    pub node: Option<Arc<str>>,
    /// `status.phase`.
    pub phase: PodPhase,
    /// What the pod asks for.
    pub resources: PodResources,
}

impl PodAllocation {
    /// Reads a Pod. See [`aggregate_pod_requests_limits`] for the errors.
    pub fn from_resource(pod: &Resource, gpus: &GpuVendors) -> Result<Self, ViewError> {
        check_kind(pod, "Pod", &[("", "Pod")])?;
        let spec = sub(pod.json(), "spec");
        Ok(Self {
            namespace: pod.meta.namespace.clone(),
            node: arc_of(spec, "nodeName"),
            phase: PodPhase::parse(str_of(sub(pod.json(), "status"), "phase")),
            resources: from_spec(spec, gpus),
        })
    }

    /// Whether the pod still holds its requests: every phase but `Succeeded` and `Failed`, whose
    /// resources the scheduler has freed. Allocation sums skip the others.
    pub fn holds_resources(&self) -> bool {
        !self.phase.is_terminal()
    }

    /// Whether the scheduler has placed the pod on a node.
    pub fn is_scheduled(&self) -> bool {
        self.node.is_some()
    }
}
