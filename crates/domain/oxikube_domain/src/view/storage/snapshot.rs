//! [`VolumeSnapshotSummary`] and [`VolumeSnapshotClassSummary`]: the CSI snapshot kinds of
//! `snapshot.storage.k8s.io`, present only where the CRDs are installed.

use std::sync::Arc;

use jiff::Timestamp;

use super::{ViewError, arc_of, check_kind, pairs_of, quantity_of, str_of, sub, ts_of};
use crate::quantity::Quantity;
use crate::resource::Resource;

/// The annotation that marks the default VolumeSnapshotClass of a driver.
pub const DEFAULT_SNAPSHOT_CLASS_ANNOTATION: &str =
    "snapshot.storage.kubernetes.io/is-default-class";

const GROUP: &str = "snapshot.storage.k8s.io";

/// One VolumeSnapshot, typed.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeSnapshotSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `metadata.namespace`.
    pub namespace: Option<Arc<str>>,
    /// `spec.source.persistentVolumeClaimName`: the claim the snapshot was taken from.
    pub source_pvc: Option<Arc<str>>,
    /// `spec.source.volumeSnapshotContentName`: a pre-provisioned snapshot's content.
    pub source_content: Option<Arc<str>>,
    /// `spec.volumeSnapshotClassName`.
    pub class: Option<Arc<str>>,
    /// `status.readyToUse`; `None` until the controller reports.
    pub ready_to_use: Option<bool>,
    /// `status.restoreSize`.
    pub restore_size: Option<Quantity>,
    /// `status.creationTime`: when the storage system took the snapshot.
    pub creation_time: Option<Timestamp>,
    /// `status.boundVolumeSnapshotContentName`.
    pub bound_content: Option<Arc<str>>,
    /// `status.error.message`.
    pub error: Option<Arc<str>>,
}

impl VolumeSnapshotSummary {
    /// Builds the summary of a VolumeSnapshot.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a `snapshot.storage.k8s.io` VolumeSnapshot.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "VolumeSnapshot", &[(GROUP, "VolumeSnapshot")])?;
        let spec = sub(res.json(), "spec");
        let source = sub(spec, "source");
        let status = sub(res.json(), "status");
        Ok(Self {
            name: res.meta.name.clone(),
            namespace: res.meta.namespace.clone(),
            source_pvc: arc_of(source, "persistentVolumeClaimName"),
            source_content: arc_of(source, "volumeSnapshotContentName"),
            class: arc_of(spec, "volumeSnapshotClassName"),
            ready_to_use: status.get("readyToUse").and_then(|v| v.as_bool()),
            restore_size: quantity_of(status, "restoreSize"),
            creation_time: ts_of(status, "creationTime"),
            bound_content: arc_of(status, "boundVolumeSnapshotContentName"),
            error: arc_of(sub(status, "error"), "message"),
        })
    }
}

/// One VolumeSnapshotClass, typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeSnapshotClassSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `driver`.
    pub driver: Option<Arc<str>>,
    /// `deletionPolicy` (`Delete` or `Retain`).
    pub deletion_policy: Option<Arc<str>>,
    /// The default-class annotation is `"true"`.
    pub is_default: bool,
    /// `parameters`, sorted by key.
    pub parameters: Vec<(Arc<str>, Arc<str>)>,
}

impl VolumeSnapshotClassSummary {
    /// Builds the summary of a VolumeSnapshotClass.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a `snapshot.storage.k8s.io`
    /// VolumeSnapshotClass.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "VolumeSnapshotClass",
            &[(GROUP, "VolumeSnapshotClass")],
        )?;
        let json = res.json();
        Ok(Self {
            name: res.meta.name.clone(),
            driver: arc_of(json, "driver"),
            deletion_policy: str_of(json, "deletionPolicy").map(Arc::from),
            is_default: res
                .meta
                .annotations
                .get(DEFAULT_SNAPSHOT_CLASS_ANNOTATION)
                .is_some_and(|v| &**v == "true"),
            parameters: pairs_of(json, "parameters"),
        })
    }
}
