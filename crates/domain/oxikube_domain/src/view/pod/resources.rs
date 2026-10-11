//! What a container asks for and exposes: CPU and memory requests and limits ([`ComputeResources`]),
//! probes ([`Probes`]) and ports ([`ContainerPort`]), and the pod's effective totals that the
//! `%CPU/R`, `%CPU/L`, `%MEM/R` and `%MEM/L` columns divide usage by.

use std::fmt;
use std::sync::Arc;

use crate::json::JsonRef;
use crate::quantity::Quantity;
use crate::view::{arc_of, arr_of, str_of, sub};

use super::status::is_restartable;

/// A CPU and a memory amount, each absent when not set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResourceAmounts {
    /// CPU, in cores (`500m` is half a core).
    pub cpu: Option<Quantity>,
    /// Memory, in bytes.
    pub memory: Option<Quantity>,
}

impl ResourceAmounts {
    fn of(v: JsonRef<'_>) -> Self {
        Self {
            cpu: v.get("cpu").and_then(quantity),
            memory: v.get("memory").and_then(quantity),
        }
    }
}

/// CPU and memory requests and limits: a container's own, or a pod's effective totals
/// ([`ComputeResources::of_pod`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComputeResources {
    /// `resources.requests`.
    pub requests: ResourceAmounts,
    /// `resources.limits`.
    pub limits: ResourceAmounts,
}

impl ComputeResources {
    /// `resources` of one container spec entry.
    pub(super) fn of_container(container: JsonRef<'_>) -> Self {
        let resources = sub(container, "resources");
        Self {
            requests: ResourceAmounts::of(sub(resources, "requests")),
            limits: ResourceAmounts::of(sub(resources, "limits")),
        }
    }

    /// The pod's effective requests and limits, as the scheduler accounts them (Kubernetes
    /// `component-helpers` `PodRequests` / `PodLimits`, without the pod overhead): pod-level
    /// `spec.resources` when set; otherwise the regular containers plus the sidecars
    /// (restartable init containers), or the largest plain init container (plus the sidecars
    /// started before it) when that is larger.
    ///
    /// A request is absent only when no container sets it (a container without one counts as
    /// zero). A limit is absent when a regular container or a sidecar has none: that container,
    /// and so the pod, is unbounded.
    pub(super) fn of_pod(spec: JsonRef<'_>) -> Self {
        let mut out = Self::of_container(spec);
        let containers: Vec<ComputeResources> = arr_of(spec, "containers")
            .iter()
            .map(Self::of_container)
            .collect();
        let inits: Vec<(bool, ComputeResources)> = arr_of(spec, "initContainers")
            .iter()
            .map(|c| (is_restartable(c), Self::of_container(c)))
            .collect();
        let pick = |amounts: fn(&ComputeResources) -> Option<Quantity>, limit: bool| {
            total(&containers, &inits, amounts, limit)
        };
        if out.requests.cpu.is_none() {
            out.requests.cpu = pick(|r| r.requests.cpu, false);
        }
        if out.requests.memory.is_none() {
            out.requests.memory = pick(|r| r.requests.memory, false);
        }
        if out.limits.cpu.is_none() {
            out.limits.cpu = pick(|r| r.limits.cpu, true);
        }
        if out.limits.memory.is_none() {
            out.limits.memory = pick(|r| r.limits.memory, true);
        }
        out
    }
}

/// One amount of the pod: `max(sum(containers) + sum(sidecars), max(init + sidecars before it))`.
/// For a limit, `None` when a container or sidecar lacks it; for a request, `None` when nobody
/// sets it.
fn total(
    containers: &[ComputeResources],
    inits: &[(bool, ComputeResources)],
    amount: fn(&ComputeResources) -> Option<Quantity>,
    limit: bool,
) -> Option<Quantity> {
    let mut any = false;
    let mut sum = Quantity::default();
    let mut sidecars = Quantity::default();
    let mut init_max = Quantity::default();
    for (restartable, r) in inits {
        let value = amount(r);
        any |= value.is_some();
        if *restartable {
            if limit && value.is_none() {
                return None;
            }
            sidecars = add(sidecars, value);
        } else {
            init_max = init_max.max(add(sidecars, value));
        }
    }
    for r in containers {
        let value = amount(r);
        if limit && value.is_none() {
            return None;
        }
        any |= value.is_some();
        sum = add(sum, value);
    }
    if !any {
        return None;
    }
    Some(add(sum, Some(sidecars)).max(init_max))
}

fn add(a: Quantity, b: Option<Quantity>) -> Quantity {
    match b {
        Some(b) => a.checked_add(&b).unwrap_or(a),
        None => a,
    }
}

/// A quantity written as a string (`"500m"`) or, rarely, as a JSON number (`1`).
fn quantity(v: JsonRef<'_>) -> Option<Quantity> {
    if let Some(s) = v.as_str() {
        return Quantity::parse(s).ok();
    }
    if let Some(n) = v.as_i64() {
        return Some(Quantity::from_value(n));
    }
    v.as_f64()
        .and_then(|f| Quantity::parse(&f.to_string()).ok())
}

/// Which probes a container defines: the `PROBES` column of k9s's containers view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Probes {
    /// `livenessProbe` is set.
    pub liveness: bool,
    /// `readinessProbe` is set.
    pub readiness: bool,
    /// `startupProbe` is set.
    pub startup: bool,
}

impl Probes {
    pub(super) fn of(container: JsonRef<'_>) -> Self {
        let set = |key| container.get(key).is_some_and(|p| p.is_object());
        Self {
            liveness: set("livenessProbe"),
            readiness: set("readinessProbe"),
            startup: set("startupProbe"),
        }
    }

    /// `L:R:S` as k9s writes it: `on` or `off` for liveness, readiness and startup
    /// (`on:on:off`). Allocates nothing.
    pub const fn display(self) -> &'static str {
        match (self.liveness, self.readiness, self.startup) {
            (false, false, false) => "off:off:off",
            (false, false, true) => "off:off:on",
            (false, true, false) => "off:on:off",
            (false, true, true) => "off:on:on",
            (true, false, false) => "on:off:off",
            (true, false, true) => "on:off:on",
            (true, true, false) => "on:on:off",
            (true, true, true) => "on:on:on",
        }
    }
}

/// One entry of a container's `ports`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContainerPort {
    /// `name`, when set.
    pub name: Option<Arc<str>>,
    /// `containerPort`.
    pub port: u16,
    /// `protocol`; `TCP` when absent (the API default).
    pub protocol: Arc<str>,
}

impl ContainerPort {
    /// Every valid entry of `ports` of one container spec entry, in order.
    pub(super) fn list(container: JsonRef<'_>) -> Arc<[ContainerPort]> {
        arr_of(container, "ports")
            .iter()
            .filter_map(|p| {
                let port = p.get("containerPort")?.as_u64()?;
                Some(ContainerPort {
                    name: arc_of(p, "name"),
                    port: u16::try_from(port).ok()?,
                    protocol: Arc::from(str_of(p, "protocol").unwrap_or("TCP")),
                })
            })
            .collect()
    }
}

impl fmt::Display for ContainerPort {
    /// `http:8080/TCP`, or `8080/UDP` without a name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = &self.name {
            write!(f, "{name}:")?;
        }
        write!(f, "{}/{}", self.port, self.protocol)
    }
}
