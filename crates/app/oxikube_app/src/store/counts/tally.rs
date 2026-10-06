//! The running health tally of a cache, and what a read of it returns.

use oxikube_domain::view::health_of;

use crate::store::StoreObject;

/// How many objects have a health verdict and how many of those are healthy. Kept by the cache.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Tally {
    pub rated: usize,
    pub healthy: usize,
}

impl Tally {
    /// Counts `object` in.
    pub fn add(&mut self, object: &StoreObject) {
        if let Some(health) = rate(object) {
            self.rated += 1;
            self.healthy += usize::from(health);
        }
    }

    /// Counts `object` out (it was added before).
    pub fn sub(&mut self, object: &StoreObject) {
        if let Some(health) = rate(object) {
            self.rated = self.rated.saturating_sub(1);
            self.healthy = self.healthy.saturating_sub(usize::from(health));
        }
    }
}

/// The verdict of one object: `None` when it has none (no rule, a Table row, metadata only).
fn rate(object: &StoreObject) -> Option<bool> {
    health_of(object.resource()?).map(|h| h.is_healthy())
}

/// One cache's totals for a part of the cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CacheTally {
    pub total: usize,
    pub rated: usize,
    pub healthy: usize,
}
