//! [`QuotaCaps`]: what the `ResourceQuota`s of a namespace cap, as exact amounts.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::quantities::{EPHEMERAL, quantity_of, whole_count};
use super::{GpuVendors, ResourceAmounts};
use crate::json::JsonRef;
use crate::quantity::Quantity;
use crate::resource::Resource;
use crate::view::{ViewError, check_kind, sub};

/// The hard cap on each resource of one side (`requests.*` or `limits.*`); `None` where no quota
/// caps it. A cap of zero is a cap (nothing may be requested), not an absence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceCaps {
    /// `requests.cpu` / `limits.cpu`.
    pub cpu: Option<Quantity>,
    /// `requests.memory` / `limits.memory`.
    pub memory: Option<Quantity>,
    /// `requests.ephemeral-storage` / `limits.ephemeral-storage`.
    pub ephemeral: Option<Quantity>,
    /// `requests.<gpu resource>` per configured GPU resource name (a quota can only cap GPU
    /// requests).
    pub gpu: BTreeMap<Arc<str>, u64>,
}

/// What a set of quotas allows in a namespace.
///
/// Where several quotas cap the same resource they all apply, so [`QuotaCaps::tightest`] keeps the
/// smallest. A quota with scopes (`BestEffort`, `Terminating`, a priority class...) caps only the
/// pods it selects: it is read ([`scoped`](Self::scoped) is set) but left out of the tightest
/// caps, because a sum over every pod of the namespace cannot be compared with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuotaCaps {
    /// Caps on the sum of requests.
    pub requests: ResourceCaps,
    /// Caps on the sum of limits.
    pub limits: ResourceCaps,
    /// `hard.pods`: how many pods may exist.
    pub pods: Option<u64>,
    /// Whether the quota has scopes or a scope selector.
    pub scoped: bool,
}

impl QuotaCaps {
    /// Reads a core `ResourceQuota`: `status.hard` (what the quota controller enforces), else
    /// `spec.hard`. Keys that name no resource of [`ResourceAmounts`] (object counts such as
    /// `count/deployments.apps`, storage classes) are ignored; so is a key whose quantity does
    /// not parse. `requests.cpu` and the bare `cpu` are the same cap, as in Kubernetes.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `quota` is not a `ResourceQuota`.
    pub fn from_resource(quota: &Resource, gpus: &GpuVendors) -> Result<Self, ViewError> {
        check_kind(quota, "ResourceQuota", &[("", "ResourceQuota")])?;
        let (spec, status) = (sub(quota.json(), "spec"), sub(quota.json(), "status"));
        let hard = match sub(status, "hard") {
            h if h.is_object() => h,
            _ => sub(spec, "hard"),
        };
        let cap = |keys: &[&str]| keys.iter().find_map(|key| quantity_of(hard, key));
        let mut caps = Self {
            requests: ResourceCaps {
                cpu: cap(&["requests.cpu", "cpu"]),
                memory: cap(&["requests.memory", "memory"]),
                ephemeral: cap(&["requests.ephemeral-storage", EPHEMERAL]),
                gpu: BTreeMap::new(),
            },
            limits: ResourceCaps {
                cpu: cap(&["limits.cpu"]),
                memory: cap(&["limits.memory"]),
                ephemeral: cap(&["limits.ephemeral-storage"]),
                gpu: BTreeMap::new(),
            },
            pods: quantity_of(hard, "pods").map(|q| whole_count(&q)),
            scoped: is_scoped(spec),
        };
        for name in gpus.names() {
            let key = format!("requests.{name}");
            if let Some(count) = quantity_of(hard, &key).map(|q| whole_count(&q)) {
                caps.requests.gpu.insert(name.clone(), count);
            }
        }
        Ok(caps)
    }

    /// The tightest caps over `quotas`: per resource, the smallest hard amount of the unscoped
    /// quotas. Entries that are not a `ResourceQuota` are skipped. Also returns how many of
    /// them were scoped and so left out.
    pub fn tightest<'a>(
        quotas: impl IntoIterator<Item = &'a Resource>,
        gpus: &GpuVendors,
    ) -> (Self, usize) {
        let mut out = Self::default();
        let mut scoped = 0;
        for quota in quotas {
            let Ok(caps) = Self::from_resource(quota, gpus) else {
                continue;
            };
            if caps.scoped {
                scoped += 1;
            } else {
                out.tighten(&caps);
            }
        }
        (out, scoped)
    }

    fn tighten(&mut self, other: &QuotaCaps) {
        self.requests.tighten(&other.requests);
        self.limits.tighten(&other.limits);
        self.pods = min_of(self.pods, other.pods);
    }

    /// Whether no quota caps anything this type models.
    pub fn is_empty(&self) -> bool {
        self.requests == ResourceCaps::default()
            && self.limits == ResourceCaps::default()
            && self.pods.is_none()
    }

    /// The request caps as amounts, with a missing cap as zero (use [`ResourceCaps`] to tell the
    /// two apart).
    pub fn request_amounts(&self) -> ResourceAmounts {
        self.requests.amounts()
    }
}

impl ResourceCaps {
    fn tighten(&mut self, other: &ResourceCaps) {
        self.cpu = min_of(self.cpu, other.cpu);
        self.memory = min_of(self.memory, other.memory);
        self.ephemeral = min_of(self.ephemeral, other.ephemeral);
        for (name, &count) in &other.gpu {
            let slot = self.gpu.entry(name.clone()).or_insert(count);
            *slot = (*slot).min(count);
        }
    }

    /// The caps as amounts, a missing cap as zero.
    pub fn amounts(&self) -> ResourceAmounts {
        ResourceAmounts {
            cpu: self.cpu.unwrap_or_default(),
            memory: self.memory.unwrap_or_default(),
            ephemeral: self.ephemeral.unwrap_or_default(),
            gpu: self.gpu.clone(),
        }
    }
}

fn min_of<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn is_scoped(spec: JsonRef<'_>) -> bool {
    let non_empty = |key: &str| {
        spec.get(key).is_some_and(|v| {
            v.as_array()
                .map_or_else(|| v.is_object(), |a| !a.is_empty())
        })
    };
    non_empty("scopes") || non_empty("scopeSelector")
}
