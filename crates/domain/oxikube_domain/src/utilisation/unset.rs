//! [`UnsetPods`]: how many pods set no request or limit, per resource.

use super::PodResources;

/// How many pods leave a resource's request or limit unset.
///
/// A total of requests is low when many pods set none, and the totals alone do not say so: this
/// is the count that does. A pod with no requests at all (`BestEffort`) is counted under every
/// resource. Counted by [`UnsetPods::add`] over the pods an allocation sum covers (the ones that
/// still hold resources).
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
        let unset = |amount: &crate::Quantity| u32::from(amount.is_zero());
        self.pods = self.pods.saturating_add(1);
        self.cpu_requests += unset(&pod.requests.cpu);
        self.cpu_limits += unset(&pod.limits.cpu);
        self.memory_requests += unset(&pod.requests.memory);
        self.memory_limits += unset(&pod.limits.memory);
        self.ephemeral_requests += unset(&pod.requests.ephemeral);
        self.ephemeral_limits += unset(&pod.limits.ephemeral);
    }
}
