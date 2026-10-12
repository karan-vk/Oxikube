//! [`AllocationKind`]: what a row of an allocation view measures.

use std::sync::Arc;

use oxikube_domain::Quantity;

/// What an [`AllocationRow`](super::AllocationRow) measures.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AllocationKind {
    /// CPU, in cores (printed in millicores).
    Cpu,
    /// Memory, in bytes.
    Memory,
    /// Ephemeral storage, in bytes.
    EphemeralStorage,
    /// The pod count.
    Pods,
    /// A GPU resource by its exact name (`nvidia.com/gpu`), a whole number.
    Gpu(Arc<str>),
}

impl AllocationKind {
    /// The row's title: `CPU`, `Memory`, `Ephemeral storage`, `Pods` or `GPU nvidia.com/gpu`.
    pub fn title(&self) -> String {
        match self {
            Self::Cpu => "CPU".to_owned(),
            Self::Memory => "Memory".to_owned(),
            Self::EphemeralStorage => "Ephemeral storage".to_owned(),
            Self::Pods => "Pods".to_owned(),
            Self::Gpu(name) => format!("GPU {name}"),
        }
    }

    /// A short stable id for the row (`cpu`, `memory`, `ephemeral`, `pods`, `gpu-nvidia.com/gpu`):
    /// what tests and element ids use.
    pub fn id(&self) -> String {
        match self {
            Self::Cpu => "cpu".to_owned(),
            Self::Memory => "memory".to_owned(),
            Self::EphemeralStorage => "ephemeral".to_owned(),
            Self::Pods => "pods".to_owned(),
            Self::Gpu(name) => format!("gpu-{name}"),
        }
    }

    /// `amount` as text in this kind's unit: `250m`, `1.5 GiB`, `12`.
    pub fn format(&self, amount: &Quantity) -> String {
        match self {
            Self::Cpu => amount.human_cpu(),
            Self::Memory | Self::EphemeralStorage => amount.human_bytes(),
            Self::Pods | Self::Gpu(_) => amount.value().to_string(),
        }
    }

    /// Whether metrics-server reports live usage of this resource (CPU and memory only).
    pub fn has_live_usage(&self) -> bool {
        matches!(self, Self::Cpu | Self::Memory)
    }
}
