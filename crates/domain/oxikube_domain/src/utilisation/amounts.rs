//! [`ResourceAmounts`]: CPU, memory, ephemeral storage and GPU counts as one summable value.

use std::collections::BTreeMap;
use std::ops::{Add, AddAssign};
use std::sync::Arc;

use crate::quantity::Quantity;

/// CPU, memory, ephemeral storage and GPU amounts.
///
/// CPU (cores), memory and ephemeral storage (bytes) are exact [`Quantity`] values; GPUs are whole counts keyed by
/// extended resource name (`nvidia.com/gpu`), because a GPU is not divisible the way a core is.
/// The zero value (all fields empty) means "nothing set". The GPU map is ordered, so equal
/// amounts compare equal and sums do not depend on the order of the terms.
///
/// Sums saturate (see [`Quantity`]'s `Add`), so adding can never panic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceAmounts {
    /// CPU in cores (`250m` is a quarter core).
    pub cpu: Quantity,
    /// Memory in bytes.
    pub memory: Quantity,
    /// Ephemeral storage (`ephemeral-storage`) in bytes.
    pub ephemeral: Quantity,
    /// GPU count per configured vendor resource name; absent names are zero.
    pub gpu: BTreeMap<Arc<str>, u64>,
}

impl ResourceAmounts {
    /// No CPU, memory or GPU.
    pub fn zero() -> Self {
        Self::default()
    }

    /// Whether every amount is zero.
    pub fn is_zero(&self) -> bool {
        self.cpu.is_zero()
            && self.memory.is_zero()
            && self.ephemeral.is_zero()
            && self.gpu.values().all(|&n| n == 0)
    }

    /// The GPU count for `resource`, zero when none.
    pub fn gpu_count(&self, resource: &str) -> u64 {
        self.gpu.get(resource).copied().unwrap_or(0)
    }

    /// The total over every GPU resource name.
    pub fn gpu_total(&self) -> u64 {
        self.gpu.values().fold(0, |a, &n| a.saturating_add(n))
    }

    /// Raises every amount to at least the corresponding amount of `other`.
    pub fn max_assign(&mut self, other: &ResourceAmounts) {
        self.cpu = self.cpu.max(other.cpu);
        self.memory = self.memory.max(other.memory);
        self.ephemeral = self.ephemeral.max(other.ephemeral);
        for (name, &count) in &other.gpu {
            if count > 0 {
                let slot = self.gpu.entry(name.clone()).or_insert(0);
                *slot = (*slot).max(count);
            }
        }
    }
}

impl AddAssign<&ResourceAmounts> for ResourceAmounts {
    fn add_assign(&mut self, rhs: &ResourceAmounts) {
        self.cpu += rhs.cpu;
        self.memory += rhs.memory;
        self.ephemeral += rhs.ephemeral;
        for (name, &count) in &rhs.gpu {
            if count == 0 {
                continue;
            }
            let slot = self.gpu.entry(name.clone()).or_insert(0);
            *slot = slot.saturating_add(count);
        }
    }
}

impl AddAssign for ResourceAmounts {
    fn add_assign(&mut self, rhs: ResourceAmounts) {
        *self += &rhs;
    }
}

impl Add for ResourceAmounts {
    type Output = ResourceAmounts;

    fn add(mut self, rhs: ResourceAmounts) -> ResourceAmounts {
        self += &rhs;
        self
    }
}

impl<'a> std::iter::Sum<&'a ResourceAmounts> for ResourceAmounts {
    fn sum<I: Iterator<Item = &'a ResourceAmounts>>(iter: I) -> Self {
        iter.fold(Self::zero(), |mut acc, x| {
            acc += x;
            acc
        })
    }
}
