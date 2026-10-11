//! [`PvcSummary`]: a PersistentVolumeClaim, typed.

use std::fmt;
use std::sync::Arc;

use jiff::Timestamp;

use super::access::AccessMode;
use super::{ViewError, arc_of, arr_of, check_kind, quantity_of, str_of, sub};
use crate::quantity::Quantity;
use crate::resource::Resource;
use crate::view::{Condition, ConditionStatus};

/// `status.phase` of a claim.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PvcPhase {
    /// Not bound to a volume yet.
    Pending,
    /// Bound to a volume.
    Bound,
    /// The volume it was bound to is gone.
    Lost,
    /// No phase, or one this version does not know.
    Unknown(Arc<str>),
}

impl PvcPhase {
    /// The API spelling (`Unknown` text as written, empty for none).
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "Pending",
            Self::Bound => "Bound",
            Self::Lost => "Lost",
            Self::Unknown(text) => text,
        }
    }
}

impl fmt::Display for PvcPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of a claim table, typed: the columns StorageClass, Size, Status and the bound volume,
/// and what the detail panel shows (conditions, resize state).
#[derive(Debug, Clone, PartialEq)]
pub struct PvcSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `metadata.namespace`.
    pub namespace: Option<Arc<str>>,
    /// `spec.storageClassName`, else the legacy `volume.beta.kubernetes.io/storage-class`
    /// annotation. `None` for a claim that names no class (it takes the default class when the
    /// API server admits it).
    pub storage_class: Option<Arc<str>>,
    /// `spec.resources.requests.storage`: what the user asked for.
    pub requested: Option<Quantity>,
    /// `status.capacity.storage`: what the bound volume gives.
    pub capacity: Option<Quantity>,
    /// `status.phase`.
    pub phase: PvcPhase,
    /// `status.accessModes` once bound, else `spec.accessModes`.
    pub access_modes: Vec<AccessMode>,
    /// `spec.volumeName`: the bound (or pre-bound) PersistentVolume.
    pub volume_name: Option<Arc<str>>,
    /// `spec.volumeMode`.
    pub volume_mode: Option<Arc<str>>,
    /// `status.conditions` (`Resizing`, `FileSystemResizePending`, ...), in API order.
    pub conditions: Vec<Condition>,
    /// `status.allocatedResourceStatuses.storage`: how far an expansion is
    /// (`ControllerResizeInProgress`, `NodeResizePending`, ...).
    pub resize_status: Option<Arc<str>>,
    /// `metadata.creationTimestamp`.
    pub created: Option<Timestamp>,
}

impl PvcSummary {
    /// Builds the summary of a core PersistentVolumeClaim.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a PersistentVolumeClaim.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "PersistentVolumeClaim",
            &[("", "PersistentVolumeClaim")],
        )?;
        let spec = sub(res.json(), "spec");
        let status = sub(res.json(), "status");
        let phase = match str_of(status, "phase") {
            Some("Pending") => PvcPhase::Pending,
            Some("Bound") => PvcPhase::Bound,
            Some("Lost") => PvcPhase::Lost,
            Some(other) => PvcPhase::Unknown(Arc::from(other)),
            None => PvcPhase::Unknown(Arc::from("")),
        };
        let modes_at = |v| -> Vec<AccessMode> {
            arr_of(v, "accessModes")
                .iter()
                .filter_map(|m| m.as_str())
                .map(AccessMode::parse)
                .collect()
        };
        let mut access_modes = modes_at(status);
        if access_modes.is_empty() {
            access_modes = modes_at(spec);
        }
        let storage_class = arc_of(spec, "storageClassName").or_else(|| {
            res.meta
                .annotations
                .get("volume.beta.kubernetes.io/storage-class")
                .filter(|class| !class.is_empty())
                .cloned()
        });
        Ok(Self {
            name: res.meta.name.clone(),
            namespace: res.meta.namespace.clone(),
            storage_class,
            requested: quantity_of(sub(sub(spec, "resources"), "requests"), "storage"),
            capacity: quantity_of(sub(status, "capacity"), "storage"),
            phase,
            access_modes,
            volume_name: arc_of(spec, "volumeName"),
            volume_mode: arc_of(spec, "volumeMode"),
            conditions: arr_of(status, "conditions")
                .iter()
                .filter_map(Condition::from_json)
                .collect(),
            resize_status: arc_of(sub(status, "allocatedResourceStatuses"), "storage"),
            created: res.meta.creation,
        })
    }

    /// The size the claim is worth to the user: the bound volume's capacity, else what was asked.
    pub fn size(&self) -> Option<Quantity> {
        self.capacity.or(self.requested)
    }

    /// The size the next resize must exceed: what was asked for (a resize already requested
    /// counts), else the capacity.
    pub fn current_request(&self) -> Option<Quantity> {
        self.requested.or(self.capacity)
    }

    /// Whether an expansion is under way: more was asked than the volume gives, or the claim
    /// carries a `Resizing` or `FileSystemResizePending` condition that holds.
    pub fn resizing(&self) -> bool {
        let asked_more = matches!((&self.requested, &self.capacity), (Some(r), Some(c)) if r > c);
        asked_more
            || self.conditions.iter().any(|c| {
                c.status == ConditionStatus::True
                    && matches!(&*c.kind, "Resizing" | "FileSystemResizePending")
            })
    }

    /// The `Size` column text: the capacity (or the request while unbound), and while a resize
    /// is pending `10Gi → 20Gi`.
    pub fn size_text(&self) -> String {
        match (&self.capacity, &self.requested) {
            (Some(capacity), Some(requested)) if requested > capacity => {
                format!("{capacity} → {requested}")
            }
            (Some(capacity), _) => capacity.to_string(),
            (None, Some(requested)) => requested.to_string(),
            (None, None) => String::new(),
        }
    }
}
