//! Utilisation maths (E13-S02): what pods, nodes and namespaces ask for and how much of it is used.
//!
//! The figures k9s and `kubectl-view-allocations` show (`%CPU/R`, `%CPU/L`, `%MEM/R`, `%MEM/L`
//! and a node's allocation) come out of a few pure functions, so the table columns, the
//! overview and the agent tools all agree on the numbers:
//!
//! | Item | Holds |
//! |---|---|
//! | [`ResourceAmounts`] | CPU and memory as exact [`Quantity`](crate::Quantity) values plus whole-number GPU counts per vendor resource name |
//! | [`GpuVendors`], [`GpuVendorsTable`] | which extended resource names count as GPUs (`metrics.gpu_vendors`), globally and per cluster |
//! | [`aggregate_pod_requests_limits`], [`PodResources`], [`PodAllocation`] | a pod's effective requests and limits, and where it is allocated |
//! | [`NodeAllocatable`] | what a node offers (`status.allocatable`) |
//! | [`percent`], [`Utilisation`], [`AmountPercents`] | usage over a base, `None` when the base is zero |
//!
//! # Rules
//!
//! All quantity maths goes through the E02 [`Quantity`](crate::Quantity) (exact, saturating);
//! there is no string or float arithmetic on quantities. The edge cases are fixed here once.
//!
//! * **Pod requests and limits** follow Kubernetes (`resourcehelper.PodRequestsAndLimits`): the
//!   sum over regular containers plus restartable init containers (sidecars); an ordinary init
//!   container runs alone with the sidecars started before it, so the pod's figure is the larger of
//!   that peak and the running total; `spec.overhead` is added to requests and limits;
//!   `spec.resources` (pod-level resources) replaces the container sum for the resources it sets.
//!   A container that sets a limit but no request already carries the request the API server
//!   defaulted, so nothing is defaulted again here. Ephemeral containers do not count (they
//!   cannot carry resources).
//! * **No requests or limits**: the amount is zero and every percent against it is `None`
//!   (shown as a dash), never `0` and never infinity.
//! * **Terminated pods** (`Succeeded`, `Failed`) hold nothing: the scheduler frees their
//!   resources, so allocation sums skip them ([`PodAllocation::holds_resources`]). Every other
//!   phase counts, `Unknown` included (the node may be unreachable but the scheduler still
//!   books it). `kubectl-view-allocations` counts only `Running` and `Pending`; the two agree
//!   except for `Unknown` pods.
//! * **Pods not yet scheduled** (`Pending`, no `spec.nodeName`) count towards their namespace
//!   (a `ResourceQuota` counts them) but towards no node ([`PodAllocation::node`] is `None`).
//! * **GPU** is an integer count, summed per configured resource name (`nvidia.com/gpu`); a
//!   fractional quantity rounds up like `Quantity::value`. Names outside the configured list
//!   are ignored, so the list decides what "GPU" means.
//!
//! # Oracle
//!
//! The semantics are those of `kubectl-view-allocations` (davidB, CC0-1.0: `collect_from_nodes`,
//! `collect_from_pods`); nothing is copied or depended on, and the tests state how each expected
//! number was derived. The init-container rule is the Kubernetes one, which that tool does not
//! model.
//!
//! # Performance
//!
//! Every function walks the JSON in place and allocates only the output. The GPU maps of
//! [`ResourceAmounts`] stay empty (no allocation) for the common pod without GPUs, so
//! [`aggregate_pod_requests_limits`] on a CPU/memory-only pod allocates nothing.

mod amounts;
mod gpu;
mod node;
mod percent;
mod pod;
mod quantities;
#[cfg(test)]
mod tests;

pub use amounts::ResourceAmounts;
pub use gpu::{DEFAULT_GPU_VENDORS, GpuVendors, GpuVendorsTable};
pub use node::NodeAllocatable;
pub use percent::{AmountPercents, Utilisation, percent};
pub use pod::{PodAllocation, PodResources, aggregate_pod_requests_limits};
