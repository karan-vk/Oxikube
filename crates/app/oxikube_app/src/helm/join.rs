//! Joining a [`ResourcePlan`] with the live objects of the [`ResourceStore`].

use std::sync::Arc;

use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::{Health, health_of};

use super::plan::ResourcePlan;
use crate::store::{Lookup, ResourceStore, StoreObject};

/// What the cluster says about one object of a release.
#[derive(Debug, Clone)]
pub enum ResourceState {
    /// The object exists. `health` is `None` for kinds without a health rule (a neutral status).
    Present {
        /// Health from the shared view-model rules (`oxikube_domain::view::health_of`).
        health: Option<Health>,
        /// The cached object, for navigation and detail.
        object: Arc<StoreObject>,
    },
    /// The manifest lists it and the cluster does not have it (deleted by hand, or pruned).
    Missing,
    /// Nothing can be said: the kind is not served (a CRD that is not installed), or its feed
    /// has not listed yet, or was refused.
    Unknown(UnknownReason),
}

/// Why a resource's state is [`ResourceState::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownReason {
    /// The cluster does not serve the kind.
    KindNotServed,
    /// The kind's feed has not listed (yet).
    NotLoaded,
}

impl ResourceState {
    /// Whether this is [`ResourceState::Missing`].
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

/// One object of a release with its live state.
#[derive(Debug, Clone)]
pub struct ReleaseResource {
    /// Identity: cluster, gvk, namespace, name.
    pub reference: ResourceRef,
    /// The template it was rendered from.
    pub source: Option<String>,
    /// The live state.
    pub state: ResourceState,
}

/// Reads every planned object from the store's open feeds: O(resources), no I/O, no
/// subscription. Call it again after the store changed.
pub fn join(plan: &ResourcePlan, store: &ResourceStore) -> Vec<ReleaseResource> {
    plan.resources
        .iter()
        .map(|planned| {
            let r = &planned.reference;
            let state = if planned.scope.is_none() {
                ResourceState::Unknown(UnknownReason::KindNotServed)
            } else {
                match store.lookup(&r.gvk, r.namespace(), &r.name) {
                    Lookup::Found(object) => ResourceState::Present {
                        health: object.resource().and_then(health_of),
                        object,
                    },
                    Lookup::Absent => ResourceState::Missing,
                    Lookup::NotLoaded => ResourceState::Unknown(UnknownReason::NotLoaded),
                }
            };
            ReleaseResource {
                reference: r.clone(),
                source: planned.source.clone(),
                state,
            }
        })
        .collect()
}
