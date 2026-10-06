//! Storage cells: PersistentVolume, PersistentVolumeClaim, StorageClass.

use jiff::Timestamp;
use oxikube_domain::Resource;
use serde_json::Value;

use super::{arr_at, spec, status_tone, str_at};
use crate::columns::{Cell, Tone};

/// `ACCESS MODES` as kubectl's short codes: `RWO`, `ROX`, `RWX`, `RWOP`.
pub(crate) fn access_modes<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let modes: Vec<&str> = arr_at(spec(res), "accessModes")
        .iter()
        .filter_map(Value::as_str)
        .map(|m| match m {
            "ReadWriteOnce" => "RWO",
            "ReadOnlyMany" => "ROX",
            "ReadWriteMany" => "RWX",
            "ReadWriteOncePod" => "RWOP",
            other => other,
        })
        .collect();
    Cell::text(modes.join(","))
}

/// `STATUS`: `status.phase` of a PV, PVC or Namespace, toned (`Bound` green, `Pending` amber, `Lost` red).
pub(crate) fn status_phase<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let phase = res.json.pointer("/status/phase").and_then(Value::as_str);
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
    let default = [
        "storageclass.kubernetes.io/is-default-class",
        "storageclass.beta.kubernetes.io/is-default-class",
    ]
    .iter()
    .any(|k| res.meta.annotations.get(*k).is_some_and(|v| &**v == "true"));
    if default {
        Cell::text("true").with_tone(Tone::Ok)
    } else {
        Cell::empty()
    }
}
