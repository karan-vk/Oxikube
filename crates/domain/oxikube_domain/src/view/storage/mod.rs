//! Storage view-models (E12-S15): PersistentVolumeClaims, PersistentVolumes, StorageClasses and
//! the CSI VolumeSnapshot kinds, plus the two pure rules the storage actions share.
//!
//! | File | Holds |
//! |---|---|
//! | `access` | [`AccessMode`] (`RWO`/`ROX`/`RWX`/`RWOP`), [`ReclaimPolicy`] |
//! | `claim` | [`PvcSummary`]: class, requested and bound size, status, bound volume, resize state |
//! | `volume` | [`PvSummary`]: capacity, modes, reclaim, claim, source, node affinity |
//! | `class` | [`StorageClassSummary`] and [`is_default_class`] (both default annotations) |
//! | `snapshot` | [`VolumeSnapshotSummary`], [`VolumeSnapshotClassSummary`] |
//! | `resize` | [`check_resize`]: new size greater than the current one and the class allows expansion |
//! | `claims` | [`pod_claims`]: the claims a Pod mounts (the input of the "used by" index) |
//!
//! Every constructor reads the raw JSON like the other view-models (ADR 0005: never
//! `k8s-openapi`), so a missing or wrongly typed field degrades to a default. Nothing here reads
//! `Secret` data; a PV's source is named by its kind and a short, non-secret detail only.

mod access;
mod claim;
mod claims;
mod class;
mod resize;
mod snapshot;
mod volume;

#[cfg(test)]
mod tests;

pub use access::{AccessMode, ReclaimPolicy, access_modes_text};
pub use claim::{PvcPhase, PvcSummary};
pub use claims::{pod_claims, pod_is_active};
pub use class::{
    BETA_DEFAULT_CLASS_ANNOTATION, DEFAULT_CLASS_ANNOTATION, StorageClassSummary, is_default_class,
};
pub use resize::{Expansion, ResizeBlock, check_resize};
pub use snapshot::{
    DEFAULT_SNAPSHOT_CLASS_ANNOTATION, VolumeSnapshotClassSummary, VolumeSnapshotSummary,
};
pub use volume::{ClaimRef, NodeAffinity, PvPhase, PvSummary, VolumeSource};

use std::sync::Arc;

use super::{ViewError, arc_of, arr_of, bool_of, check_kind, obj_of, str_of, sub, ts_of};
use crate::json::JsonRef;
use crate::quantity::Quantity;

/// `v[key]` parsed as a quantity; `None` when absent or not one.
fn quantity_of(v: JsonRef<'_>, key: &str) -> Option<Quantity> {
    Quantity::parse(str_of(v, key)?).ok()
}

/// The strings of `v[key]`, an array of strings.
fn strings_of(v: JsonRef<'_>, key: &str) -> Vec<Arc<str>> {
    arr_of(v, key)
        .iter()
        .filter_map(JsonRef::as_str)
        .map(Arc::from)
        .collect()
}

/// The string members of the object `v[key]` as sorted `(key, value)` pairs.
fn pairs_of(v: JsonRef<'_>, key: &str) -> Vec<(Arc<str>, Arc<str>)> {
    let mut pairs: Vec<(Arc<str>, Arc<str>)> = obj_of(v, key)
        .and_then(JsonRef::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(k, v)| Some((Arc::from(k), Arc::from(v.as_str()?))))
                .collect()
        })
        .unwrap_or_default();
    pairs.sort();
    pairs
}
