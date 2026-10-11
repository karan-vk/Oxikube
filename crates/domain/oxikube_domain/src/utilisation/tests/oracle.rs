//! An independent model of `kubectl-view-allocations` semantics, and a hand-derived table.
//!
//! `kubectl-view-allocations` (davidB, CC0-1.0) sums `spec.containers[].resources.{requests,
//! limits}` per pod with its `Qty` type (exact, `m`/`Ki`/... aware), skips pods that are not
//! `Running` or `Pending`, and books the pod to `spec.nodeName`. The model below does exactly
//! that with plain integers (nano-cores, bytes), one line per rule, so it shares no code with the
//! implementation. Where the implementation is richer (init containers, sidecars, overhead) the
//! model adds the Kubernetes rule in its simplest form.

use serde_json::json;

use super::{container, pod, q};
use crate::utilisation::{GpuVendors, PodAllocation, aggregate_pod_requests_limits};

/// One container of the model: nano-cores and bytes, `0` meaning unset.
#[derive(Clone, Copy, Default)]
pub(super) struct Ref {
    pub cpu_req: i128,
    pub cpu_lim: i128,
    pub mem_req: i128,
    pub mem_lim: i128,
}

/// The model's pod: regular containers summed, then `max` with each init container alone.
pub(super) fn model(containers: &[Ref], inits: &[Ref]) -> Ref {
    let mut sum = Ref::default();
    for c in containers {
        sum.cpu_req += c.cpu_req;
        sum.cpu_lim += c.cpu_lim;
        sum.mem_req += c.mem_req;
        sum.mem_lim += c.mem_lim;
    }
    for i in inits {
        sum.cpu_req = sum.cpu_req.max(i.cpu_req);
        sum.cpu_lim = sum.cpu_lim.max(i.cpu_lim);
        sum.mem_req = sum.mem_req.max(i.mem_req);
        sum.mem_lim = sum.mem_lim.max(i.mem_lim);
    }
    sum
}

/// Whether the model books the pod: `Running` or `Pending` (the tool's `is_scheduled`).
pub(super) fn model_counts(phase: &str) -> bool {
    matches!(phase, "Running" | "Pending")
}

/// Hand-derived fixtures. Each expected value is written next to the arithmetic that gives it,
/// the way `kubectl-view-allocations` would print it (`Requested` / `Limit` columns).
#[test]
fn the_fixture_table_matches_the_tool_semantics() {
    struct Case {
        name: &'static str,
        spec: serde_json::Value,
        /// (cpu req, cpu lim, mem req, mem lim) as the tool prints them.
        want: (&'static str, &'static str, &'static str, &'static str),
    }
    let cases = [
        Case {
            // 100m + 200m = 300m; 64Mi + 64Mi = 128Mi; limits 500m / 256Mi on one container only.
            name: "two containers",
            spec: json!({"containers": [
                container("a", json!({"cpu": "100m", "memory": "64Mi"}), json!({"cpu": "500m", "memory": "256Mi"})),
                container("b", json!({"cpu": "200m", "memory": "64Mi"}), json!(null)),
            ]}),
            want: ("300m", "500m", "128Mi", "256Mi"),
        },
        Case {
            // 1.5 cores written three ways (1500m = 1.5 = 1500000u) add to 4500m; 1Gi + 1024Mi = 2Gi.
            name: "spellings of one value",
            spec: json!({"containers": [
                container("a", json!({"cpu": "1500m", "memory": "1Gi"}), json!(null)),
                container("b", json!({"cpu": "1.5", "memory": "1024Mi"}), json!(null)),
                container("c", json!({"cpu": "1500000u", "memory": "0"}), json!(null)),
            ]}),
            want: ("4500m", "0", "2Gi", "0"),
        },
        Case {
            // Decimal and binary memory mix: 1G (10^9) + 1Gi (2^30) = 2073741824.
            name: "decimal and binary memory",
            spec: json!({"containers": [
                container("a", json!({"memory": "1G"}), json!(null)),
                container("b", json!({"memory": "1Gi"}), json!(null)),
            ]}),
            want: ("0", "0", "2073741824", "0"),
        },
        Case {
            // No resources at all: the tool prints nothing for the pod; zero here.
            name: "best effort",
            spec: json!({"containers": [container("a", json!(null), json!(null))]}),
            want: ("0", "0", "0", "0"),
        },
    ];
    for case in cases {
        let p = pod("p", "ns", Some("n"), "Running", case.spec);
        let r = aggregate_pod_requests_limits(&p, &GpuVendors::default()).unwrap();
        assert_eq!(r.requests.cpu, q(case.want.0), "{} cpu req", case.name);
        assert_eq!(r.limits.cpu, q(case.want.1), "{} cpu lim", case.name);
        assert_eq!(r.requests.memory, q(case.want.2), "{} mem req", case.name);
        assert_eq!(r.limits.memory, q(case.want.3), "{} mem lim", case.name);
    }
}

#[test]
fn phases_are_booked_like_the_tool_except_unknown() {
    let spec = json!({"containers": [container("a", json!({"cpu": "1"}), json!(null))]});
    for phase in ["Running", "Pending", "Succeeded", "Failed"] {
        let a = PodAllocation::from_resource(
            &pod("p", "ns", Some("n"), phase, spec.clone()),
            &GpuVendors::default(),
        )
        .unwrap();
        assert_eq!(a.holds_resources(), model_counts(phase), "{phase}");
    }
    // The one divergence, on purpose: the scheduler still books an `Unknown` pod.
    let unknown = PodAllocation::from_resource(
        &pod("p", "ns", Some("n"), "Unknown", spec),
        &GpuVendors::default(),
    )
    .unwrap();
    assert!(unknown.holds_resources() && !model_counts("Unknown"));
}
