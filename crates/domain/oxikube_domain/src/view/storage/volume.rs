//! [`PvSummary`]: a PersistentVolume, typed.

use std::fmt;
use std::sync::Arc;

use jiff::Timestamp;

use super::access::{AccessMode, ReclaimPolicy};
use super::{ViewError, arc_of, arr_of, check_kind, quantity_of, str_of, strings_of, sub};
use crate::json::JsonRef;
use crate::quantity::Quantity;
use crate::resource::Resource;

/// `status.phase` of a volume.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PvPhase {
    /// Free, not bound to a claim.
    Available,
    /// Bound to a claim.
    Bound,
    /// The claim was deleted; the volume waits to be reclaimed.
    Released,
    /// Reclaiming failed.
    Failed,
    /// Not yet available.
    Pending,
    /// No phase, or one this version does not know.
    Unknown(Arc<str>),
}

impl PvPhase {
    /// The API spelling.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Available => "Available",
            Self::Bound => "Bound",
            Self::Released => "Released",
            Self::Failed => "Failed",
            Self::Pending => "Pending",
            Self::Unknown(text) => text,
        }
    }
}

impl fmt::Display for PvPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `spec.claimRef`: the claim a volume is bound (or reserved) for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClaimRef {
    /// The claim's namespace.
    pub namespace: Option<Arc<str>>,
    /// The claim's name.
    pub name: Arc<str>,
}

impl fmt::Display for ClaimRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.namespace {
            Some(ns) => write!(f, "{ns}/{}", self.name),
            None => f.write_str(&self.name),
        }
    }
}

/// Where a volume's storage comes from: the kind of source and one short, non-secret detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeSource {
    /// The spec field that names it (`csi`, `nfs`, `hostPath`, `local`, `awsElasticBlockStore`, ...).
    pub kind: Arc<str>,
    /// The CSI driver, the NFS `server:path`, the host path, ...: what identifies the backing
    /// storage. Never a credential (secret references are not read).
    pub detail: Option<String>,
}

/// `spec.nodeAffinity.required`: the node selector terms a volume is usable on. The terms are
/// ORed, the expressions inside a term ANDed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NodeAffinity {
    /// One entry per term, each a list of `key In a,b` expressions.
    pub terms: Vec<Vec<String>>,
}

impl NodeAffinity {
    /// One line per term: `topology.kubernetes.io/zone In eu-1a AND kubernetes.io/os In linux`.
    pub fn lines(&self) -> Vec<String> {
        self.terms.iter().map(|t| t.join(" AND ")).collect()
    }
}

/// One PersistentVolume, typed.
#[derive(Debug, Clone, PartialEq)]
pub struct PvSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `spec.capacity.storage`.
    pub capacity: Option<Quantity>,
    /// `spec.accessModes`.
    pub access_modes: Vec<AccessMode>,
    /// `spec.persistentVolumeReclaimPolicy` (`Retain` when the volume names none).
    pub reclaim_policy: ReclaimPolicy,
    /// `status.phase`.
    pub phase: PvPhase,
    /// `spec.claimRef`.
    pub claim: Option<ClaimRef>,
    /// `spec.storageClassName`.
    pub storage_class: Option<Arc<str>>,
    /// `spec.volumeMode`.
    pub volume_mode: Option<Arc<str>>,
    /// `status.reason` (why a volume failed).
    pub reason: Option<Arc<str>>,
    /// `status.message`.
    pub message: Option<Arc<str>>,
    /// The backing storage.
    pub source: Option<VolumeSource>,
    /// `spec.nodeAffinity.required`.
    pub node_affinity: NodeAffinity,
    /// `spec.mountOptions`.
    pub mount_options: Vec<Arc<str>>,
    /// `metadata.creationTimestamp`.
    pub created: Option<Timestamp>,
}

/// The `spec` members that are not a volume source.
const NOT_A_SOURCE: &[&str] = &[
    "accessModes",
    "capacity",
    "claimRef",
    "mountOptions",
    "nodeAffinity",
    "persistentVolumeReclaimPolicy",
    "storageClassName",
    "volumeMode",
    "volumeAttributesClassName",
];

impl PvSummary {
    /// Builds the summary of a core PersistentVolume.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a PersistentVolume.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "PersistentVolume", &[("", "PersistentVolume")])?;
        let spec = sub(res.json(), "spec");
        let status = sub(res.json(), "status");
        let phase = match str_of(status, "phase") {
            Some("Available") => PvPhase::Available,
            Some("Bound") => PvPhase::Bound,
            Some("Released") => PvPhase::Released,
            Some("Failed") => PvPhase::Failed,
            Some("Pending") => PvPhase::Pending,
            Some(other) => PvPhase::Unknown(Arc::from(other)),
            None => PvPhase::Unknown(Arc::from("")),
        };
        let claim = spec.get("claimRef").and_then(|c| {
            Some(ClaimRef {
                namespace: arc_of(c, "namespace"),
                name: arc_of(c, "name")?,
            })
        });
        Ok(Self {
            name: res.meta.name.clone(),
            capacity: quantity_of(sub(spec, "capacity"), "storage"),
            access_modes: arr_of(spec, "accessModes")
                .iter()
                .filter_map(|m| m.as_str())
                .map(AccessMode::parse)
                .collect(),
            reclaim_policy: str_of(spec, "persistentVolumeReclaimPolicy")
                .map_or(ReclaimPolicy::Retain, ReclaimPolicy::parse),
            phase,
            claim,
            storage_class: arc_of(spec, "storageClassName"),
            volume_mode: arc_of(spec, "volumeMode"),
            reason: arc_of(status, "reason"),
            message: arc_of(status, "message"),
            source: source_of(spec),
            node_affinity: affinity_of(spec),
            mount_options: strings_of(spec, "mountOptions"),
            created: res.meta.creation,
        })
    }
}

/// The first `spec` member that is a source, with its identifying detail.
fn source_of(spec: JsonRef<'_>) -> Option<VolumeSource> {
    let (kind, body) = spec
        .as_object()?
        .iter()
        .find(|(key, _)| !NOT_A_SOURCE.contains(key))?;
    let text = |key: &str| str_of(body, key).map(str::to_owned);
    let detail = match kind {
        "csi" => text("driver"),
        "nfs" => match (text("server"), text("path")) {
            (Some(server), Some(path)) => Some(format!("{server}:{path}")),
            (server, path) => server.or(path),
        },
        "hostPath" | "local" => text("path"),
        "iscsi" => text("targetPortal"),
        "awsElasticBlockStore" | "gcePersistentDisk" | "azureDisk" => text("volumeID")
            .or_else(|| text("pdName"))
            .or_else(|| text("diskName")),
        _ => None,
    };
    Some(VolumeSource {
        kind: Arc::from(kind),
        detail,
    })
}

/// The required node selector terms, rendered.
fn affinity_of(spec: JsonRef<'_>) -> NodeAffinity {
    let required = sub(sub(spec, "nodeAffinity"), "required");
    let terms = arr_of(required, "nodeSelectorTerms")
        .iter()
        .map(|term| {
            ["matchExpressions", "matchFields"]
                .into_iter()
                .flat_map(|list| arr_of(term, list).iter())
                .filter_map(|expr| {
                    let key = str_of(expr, "key")?;
                    let op = str_of(expr, "operator").unwrap_or("In");
                    let values: Vec<&str> = arr_of(expr, "values")
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect();
                    Some(if values.is_empty() {
                        format!("{key} {op}")
                    } else {
                        format!("{key} {op} {}", values.join(","))
                    })
                })
                .collect::<Vec<_>>()
        })
        .filter(|term| !term.is_empty())
        .collect();
    NodeAffinity { terms }
}
