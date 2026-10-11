//! Storage cells: PersistentVolume, PersistentVolumeClaim, StorageClass, VolumeSnapshot and its
//! class.

use jiff::Timestamp;
use oxikube_domain::json::JsonRef;
use oxikube_domain::view::storage::{AccessMode, is_default_class};
use oxikube_domain::{Quantity, Resource};

use super::{arr_at, spec, status, status_tone, str_at};
use crate::columns::{Cell, Tone};

/// `ACCESS MODES` as kubectl's short codes: `RWO`, `ROX`, `RWX`, `RWOP`.
pub(crate) fn access_modes<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let modes: Vec<String> = arr_at(spec(res), "accessModes")
        .iter()
        .filter_map(JsonRef::as_str)
        .map(|m| AccessMode::parse(m).short().to_owned())
        .collect();
    Cell::text(modes.join(","))
}

/// `SIZE` of a claim: the bound volume's capacity, else what was asked for, and while an
/// expansion is pending `10Gi → 20Gi` (capacity, then request). Sorts by the size it shows last.
pub(crate) fn pvc_size<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let text_at = |ptr| res.json().pointer(ptr).and_then(JsonRef::as_str);
    let capacity = text_at("/status/capacity/storage").filter(|t| Quantity::parse(t).is_ok());
    let requested =
        text_at("/spec/resources/requests/storage").filter(|t| Quantity::parse(t).is_ok());
    let quantity = |t: &str| Quantity::parse(t).unwrap_or_default();
    match (capacity, requested) {
        (Some(c), Some(r)) if quantity(r) > quantity(c) => {
            Cell::quantity(format!("{c} → {r}"), quantity(r)).with_tone(Tone::Warn)
        }
        (Some(t), _) | (None, Some(t)) => Cell::quantity(t, quantity(t)),
        (None, None) => Cell::empty(),
    }
}

/// `STATUS`: `status.phase` of a PV, PVC or Namespace, toned (`Bound` green, `Pending` amber, `Lost` red).
pub(crate) fn status_phase<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let phase = res
        .json()
        .pointer("/status/phase")
        .and_then(JsonRef::as_str);
    phase.map_or_else(Cell::empty, |p| Cell::text(p).with_tone(status_tone(p)))
}

/// `CLAIM` of a PV: `namespace/name` of `spec.claimRef`.
pub(crate) fn pv_claim<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(claim) = spec(res).get("claimRef") else {
        return Cell::empty();
    };
    match (str_at(claim, "namespace"), str_at(claim, "name")) {
        (Some(ns), Some(name)) => Cell::text(format!("{ns}/{name}")),
        (None, Some(name)) => Cell::text(name),
        _ => Cell::empty(),
    }
}

/// `DEFAULT` of a StorageClass: `true` when it carries the default-class annotation.
pub(crate) fn storage_class_default<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    if is_default_class(&res.meta) {
        Cell::text("default").with_tone(Tone::Ok)
    } else {
        Cell::empty()
    }
}

/// `READY` of a VolumeSnapshot: `status.readyToUse`, blank until the controller reports.
pub(crate) fn snapshot_ready<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    match status(res).get("readyToUse").and_then(JsonRef::as_bool) {
        Some(true) => Cell::text("true").with_tone(Tone::Ok),
        Some(false) => Cell::text("false").with_tone(Tone::Warn),
        None => Cell::empty(),
    }
}

/// `SOURCE` of a VolumeSnapshot: the claim it was taken from, else the pre-provisioned content.
pub(crate) fn snapshot_source<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let source = spec(res).get("source").unwrap_or(JsonRef::NULL);
    str_at(source, "persistentVolumeClaimName")
        .or_else(|| str_at(source, "volumeSnapshotContentName"))
        .map_or_else(Cell::empty, Cell::text)
}

/// `CREATED` of a VolumeSnapshot: `status.creationTime`, when the storage system took it.
pub(crate) fn snapshot_created<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(text) = str_at(status(res), "creationTime") else {
        return Cell::empty();
    };
    text.parse::<Timestamp>()
        .map_or_else(|_| Cell::text(text), |at| Cell::time(text, at))
}

/// `DEFAULT` of a VolumeSnapshotClass: the driver's default-class annotation.
pub(crate) fn snapshot_class_default<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let default = res
        .meta
        .annotations
        .get(oxikube_domain::view::storage::DEFAULT_SNAPSHOT_CLASS_ANNOTATION)
        .is_some_and(|v| &**v == "true");
    if default {
        Cell::text("default").with_tone(Tone::Ok)
    } else {
        Cell::empty()
    }
}
