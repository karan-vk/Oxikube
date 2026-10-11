//! What a node offers.

use std::sync::Arc;

use super::quantities::{amounts_of, quantity_of, whole_count};
use super::{GpuVendors, ResourceAmounts};
use crate::resource::Resource;
use crate::view::{ViewError, check_kind, sub};

/// A node's `status.allocatable`: what pods can be given (capacity minus system reservations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeAllocatable {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// Allocatable CPU, memory and GPU counts (only the configured GPU resources).
    pub allocatable: ResourceAmounts,
    /// `status.allocatable.pods`: how many pods the node takes. `None` when not reported.
    pub max_pods: Option<u64>,
}

impl NodeAllocatable {
    /// Reads a Node.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `node` is not a core `Node`. Missing fields are zero / `None`.
    pub fn from_resource(node: &Resource, gpus: &GpuVendors) -> Result<Self, ViewError> {
        check_kind(node, "Node", &[("", "Node")])?;
        let allocatable = sub(sub(node.json(), "status"), "allocatable");
        Ok(Self {
            name: node.meta.name.clone(),
            allocatable: amounts_of(allocatable, gpus),
            max_pods: quantity_of(allocatable, "pods").map(|q| whole_count(&q)),
        })
    }
}
