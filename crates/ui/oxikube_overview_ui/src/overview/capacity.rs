//! The capacity lines of the view-model: per resource what the nodes can allocate, what the pods
//! on them request and are limited to, and what they use now, from an [`OverviewSnapshot`]. Pure,
//! built once per publish.

use oxikube_app::cluster_overview::OverviewSnapshot;
use oxikube_domain::AllocationResource;

/// Why the pods' requests and limits are not known: the watch budget lists the pods without their
/// spec.
pub const PODS_PARTIAL: &str = "requests not known: the watch budget lists pods without their spec";
/// Why the nodes' allocatable is not known: the watch budget lists the nodes without their status.
pub const NODES_PARTIAL: &str =
    "allocatable not known: the watch budget lists nodes without their status";

/// One capacity bar: what the nodes can allocate, what the pods on them request and are limited
/// to, and what they use now. A figure that cannot be read is `None`, drawn as `–`, never a zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityLine {
    /// CPU (millicores), memory (bytes) or pods.
    pub resource: AllocationResource,
    /// The nodes' allocatable sum; `None` before the nodes are listed, or when the watch budget
    /// lists them without their status.
    pub allocatable: Option<i64>,
    /// The pods' requests (for pods: how many occupy a node); `None` before the pods are listed,
    /// or when the watch budget lists them without their spec.
    pub requests: Option<i64>,
    /// The pods' limits; `None` for the pod count, or when the requests are not known.
    pub limits: Option<i64>,
    /// Live usage from metrics-server; `None` without it (never a zero).
    pub usage: Option<i64>,
    /// Why a figure is not known, when the watch budget degraded a feed.
    pub missing: Option<&'static str>,
}

fn fraction(amount: i64, total: Option<i64>) -> f32 {
    match total.filter(|t| *t > 0) {
        Some(total) => (amount.max(0) as f64 / total as f64).clamp(0.0, 1.0) as f32,
        None => 0.0,
    }
}

fn percent(amount: i64, total: Option<i64>) -> Option<u32> {
    let total = total.filter(|t| *t > 0)?;
    u32::try_from(i128::from(amount.max(0)) * 100 / i128::from(total)).ok()
}

impl CapacityLine {
    /// Where the requests fill reaches, in `0..=1` of allocatable (`0` when either is unknown).
    pub fn requests_fill(&self) -> f32 {
        self.requests
            .map_or(0.0, |requests| fraction(requests, self.allocatable))
    }

    /// Whether the requests are known to exceed allocatable.
    pub fn overcommitted(&self) -> bool {
        self.requests
            .zip(self.allocatable)
            .is_some_and(|(requests, allocatable)| requests > allocatable)
    }

    /// Where the limits mark sits, in `0..=1` (clamped: the text says the real figure).
    pub fn limits_fill(&self) -> Option<f32> {
        self.limits.map(|l| fraction(l, self.allocatable))
    }

    /// Where the usage bar reaches, in `0..=1`.
    pub fn usage_fill(&self) -> Option<f32> {
        self.usage.map(|u| fraction(u, self.allocatable))
    }

    /// The requests as a percent of allocatable.
    pub fn requests_percent(&self) -> Option<u32> {
        percent(self.requests?, self.allocatable)
    }

    /// `1.5 / 6 cores`-style summary: requests (and usage) against allocatable, and why a figure
    /// is missing when one is.
    pub fn summary(&self) -> String {
        let fmt = |n| self.resource.format(n);
        let of = self.allocatable.map_or_else(|| "–".to_owned(), fmt);
        let with_pct = |n, label: &str| match percent(n, self.allocatable) {
            Some(p) => format!("{label} {} ({p}%)", fmt(n)),
            None => format!("{label} {}", fmt(n)),
        };
        let mut parts = Vec::new();
        if let Some(usage) = self.usage {
            parts.push(with_pct(usage, "used"));
        }
        let requested = if self.resource == AllocationResource::Pods {
            "scheduled"
        } else {
            "requested"
        };
        parts.push(match self.requests {
            Some(requests) => with_pct(requests, requested),
            None => format!("{requested} –"),
        });
        if let Some(limits) = self.limits {
            parts.push(with_pct(limits, "limits"));
        }
        let text = format!("{} of {of}", parts.join(" · "));
        match self.missing {
            Some(missing) => format!("{text} ({missing})"),
            None => text,
        }
    }
}

/// The CPU, memory and pod lines. A feed the watch budget degraded (metadata or table rows)
/// gives no requests or allocatable: those read `–` with the reason, not `0`.
pub fn capacity_lines(s: &OverviewSnapshot) -> [CapacityLine; 3] {
    let nodes_known = s.nodes_feed.is_listed() && !s.nodes_feed.partial;
    let pods_known = s.pods_feed.is_listed() && !s.pods_feed.partial;
    let alloc = |n: i64| nodes_known.then_some(n);
    let demand = |n: i64| pods_known.then_some(n);
    let missing = if s.pods_feed.partial {
        Some(PODS_PARTIAL)
    } else if s.nodes_feed.partial {
        Some(NODES_PARTIAL)
    } else {
        None
    };
    [
        CapacityLine {
            resource: AllocationResource::Cpu,
            allocatable: alloc(s.capacity.cpu_millis),
            requests: demand(s.demand.cpu_requests),
            limits: demand(s.demand.cpu_limits),
            usage: s.usage.map(|u| u.cpu_millis()),
            missing,
        },
        CapacityLine {
            resource: AllocationResource::Memory,
            allocatable: alloc(s.capacity.memory_bytes),
            requests: demand(s.demand.memory_requests),
            limits: demand(s.demand.memory_limits),
            usage: s.usage.map(|u| u.memory()),
            missing,
        },
        CapacityLine {
            resource: AllocationResource::Pods,
            allocatable: alloc(s.capacity.pods),
            requests: demand(i64::from(s.demand.pods)),
            limits: None,
            usage: None,
            missing,
        },
    ]
}
