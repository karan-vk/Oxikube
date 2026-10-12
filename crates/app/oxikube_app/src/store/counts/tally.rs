//! The running health tally of a cache, and what a read of it returns.

use oxikube_domain::view::{Bucket, bucket_of, health_of};

use super::BucketCounts;
use crate::store::StoreObject;

/// How many objects have a health verdict, how many of those are healthy and how they spread over
/// the status buckets. Kept by the cache, O(1) per delta.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Tally {
    pub rated: usize,
    pub healthy: usize,
    pub buckets: BucketCounts,
}

impl Tally {
    /// Counts `object` in.
    pub fn add(&mut self, object: &StoreObject) {
        if let Some((healthy, bucket)) = rate(object) {
            self.rated += 1;
            self.healthy += usize::from(healthy);
            self.buckets.add(bucket);
        }
    }

    /// Counts `object` out (it was added before).
    pub fn sub(&mut self, object: &StoreObject) {
        if let Some((healthy, bucket)) = rate(object) {
            self.rated = self.rated.saturating_sub(1);
            self.healthy = self.healthy.saturating_sub(usize::from(healthy));
            self.buckets.sub(bucket);
        }
    }
}

/// The verdict and bucket of one object: `None` when it has none (no rule, a Table row, metadata
/// only). Reads the object's JSON once for the verdict; the bucket refines it.
fn rate(object: &StoreObject) -> Option<(bool, Bucket)> {
    let resource = object.resource()?;
    let health = health_of(resource)?;
    Some((health.is_healthy(), bucket_of(resource, health)))
}
