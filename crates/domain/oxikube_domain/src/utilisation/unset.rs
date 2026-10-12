//! [`UnsetPods`]: how many pods set no request or limit, per resource.

use super::{PodResources, ResourceAmounts};

/// Which requests and limits a pod's containers (and its pod-level resources) declare, as
/// non-zero amounts, before the runtime class's overhead is added.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Declared {
    /// A CPU request is declared.
    pub cpu_requests: bool,
    /// A CPU limit is declared.
    pub cpu_limits: bool,
    /// A memory request is declared.
    pub memory_requests: bool,
    /// A memory limit is declared.
    pub memory_limits: bool,
    /// An ephemeral-storage request is declared.
    pub ephemeral_requests: bool,
    /// An ephemeral-storage limit is declared.
    pub ephemeral_limits: bool,
}

impl Declared {
    pub(super) fn of(requests: &ResourceAmounts, limits: &ResourceAmounts) -> Self {
        Self {
            cpu_requests: !requests.cpu.is_zero(),
            cpu_limits: !limits.cpu.is_zero(),
            memory_requests: !requests.memory.is_zero(),
            memory_limits: !limits.memory.is_zero(),
            ephemeral_requests: !requests.ephemeral.is_zero(),
            ephemeral_limits: !limits.ephemeral.is_zero(),
        }
    }
}

/// How many pods leave a resource's request or limit unset.
///
/// A total of requests is low when many pods set none, and the totals alone do not say so: this
/// is the count that does. A pod with no requests at all (`BestEffort`) is counted under every
/// resource. Counted by [`UnsetPods::add`] over the pods an allocation sum covers (the ones that
/// still hold resources), from what the pod itself declares ([`Declared`]): a runtime class's
/// overhead does not make a pod "set" a request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnsetPods {
    /// Pods counted.
    pub pods: u32,
    /// Pods with no CPU request.
    pub cpu_requests: u32,
    /// Pods with no CPU limit.
    pub cpu_limits: u32,
    /// Pods with no memory request.
    pub memory_requests: u32,
    /// Pods with no memory limit.
    pub memory_limits: u32,
    /// Pods with no ephemeral-storage request.
    pub ephemeral_requests: u32,
    /// Pods with no ephemeral-storage limit.
    pub ephemeral_limits: u32,
}

impl UnsetPods {
    /// Counts one pod.
    pub fn add(&mut self, pod: &PodResources) {
        let unset = |declared: bool| u32::from(!declared);
        let d = &pod.declared;
        self.pods = self.pods.saturating_add(1);
        self.cpu_requests += unset(d.cpu_requests);
        self.cpu_limits += unset(d.cpu_limits);
        self.memory_requests += unset(d.memory_requests);
        self.memory_limits += unset(d.memory_limits);
        self.ephemeral_requests += unset(d.ephemeral_requests);
        self.ephemeral_limits += unset(d.ephemeral_limits);
    }
}
