//! [`GpuVendorsHandle`]: the live `metrics.gpu_vendors` table.

use std::sync::Arc;

use oxikube_domain::ids::ClusterId;
use oxikube_domain::utilisation::{GpuVendors, GpuVendorsTable};
use parking_lot::RwLock;

/// A shared, replaceable [`GpuVendorsTable`].
///
/// `oxikube_app` has no settings crate, so the binary resolves `metrics.gpu_vendors` (global and
/// per cluster) and calls [`GpuVendorsHandle::set`] now and on every change. Readers (the
/// aggregator) take the list for a cluster at the moment they compute, so an edit applies to the
/// next computation of every open cluster without a restart. Cloning shares the same table.
#[derive(Debug, Clone, Default)]
pub struct GpuVendorsHandle {
    table: Arc<RwLock<Arc<GpuVendorsTable>>>,
}

impl GpuVendorsHandle {
    /// A handle holding `table`.
    pub fn new(table: GpuVendorsTable) -> Self {
        Self {
            table: Arc::new(RwLock::new(Arc::new(table))),
        }
    }

    /// Replaces the table; returns whether it changed.
    pub fn set(&self, table: GpuVendorsTable) -> bool {
        let mut slot = self.table.write();
        if **slot == table {
            return false;
        }
        *slot = Arc::new(table);
        true
    }

    /// The list for `cluster` now. A short read lock and an `Arc` clone; never blocks on I/O.
    pub fn for_cluster(&self, cluster: &ClusterId) -> GpuVendors {
        self.table.read().for_cluster(cluster).clone()
    }

    /// The whole table now.
    pub fn table(&self) -> Arc<GpuVendorsTable> {
        self.table.read().clone()
    }
}
