//! Volumes and the mounts of a container.

use std::sync::Arc;

use crate::json::JsonRef;
use crate::view::{arc_of, arr_of, bool_of, str_of, sub};

/// One entry of a container's `volumeMounts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountSummary {
    /// The volume's name.
    pub volume: Arc<str>,
    /// `mountPath`.
    pub path: Arc<str>,
    /// `readOnly`.
    pub read_only: bool,
    /// `subPath`, when set.
    pub sub_path: Option<Arc<str>>,
}

impl MountSummary {
    /// `/etc/config (ro)` and so on: the path with its flags.
    pub fn flags(&self) -> String {
        let mut out = String::new();
        if let Some(sub_path) = &self.sub_path {
            out.push_str(&format!("subPath {sub_path}"));
        }
        if self.read_only {
            if !out.is_empty() {
                out.push_str(", ");
            }
            out.push_str("read-only");
        }
        out
    }
}

pub(super) fn mounts_of(container: JsonRef<'_>) -> Vec<MountSummary> {
    arr_of(container, "volumeMounts")
        .iter()
        .filter_map(|mount| {
            Some(MountSummary {
                volume: arc_of(mount, "name")?,
                path: arc_of(mount, "mountPath")?,
                read_only: bool_of(mount, "readOnly"),
                sub_path: arc_of(mount, "subPath"),
            })
        })
        .collect()
}

/// One entry of `spec.volumes`: its name, what backs it, and what names the backing object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeSummary {
    /// The volume's name.
    pub name: Arc<str>,
    /// What backs it: `ConfigMap`, `Secret`, `PersistentVolumeClaim`, `EmptyDir`, `HostPath`,
    /// `Projected`, `DownwardAPI`, `CSI`..., or the spec's own key for another source.
    pub source: Arc<str>,
    /// The object or path behind it (`web-config`, `/var/log`, `2 sources`); empty when there is
    /// nothing to name.
    pub detail: String,
}

/// Source keys with the field that names the object behind them.
const NAMED: &[(&str, &str, &str)] = &[
    ("configMap", "ConfigMap", "name"),
    ("secret", "Secret", "secretName"),
    (
        "persistentVolumeClaim",
        "PersistentVolumeClaim",
        "claimName",
    ),
    ("hostPath", "HostPath", "path"),
    ("nfs", "NFS", "server"),
    ("csi", "CSI", "driver"),
    ("iscsi", "iSCSI", "targetPortal"),
    ("downwardAPI", "DownwardAPI", ""),
    ("image", "Image", "reference"),
];

pub(super) fn volumes_of(spec: JsonRef<'_>) -> Vec<VolumeSummary> {
    arr_of(spec, "volumes")
        .iter()
        .filter_map(|volume| {
            let name = arc_of(volume, "name")?;
            Some(describe(name, volume))
        })
        .collect()
}

fn describe(name: Arc<str>, volume: JsonRef<'_>) -> VolumeSummary {
    for (key, source, field) in NAMED {
        if let Some(body) = volume.get(key).filter(|b| b.is_object()) {
            return VolumeSummary {
                name,
                source: Arc::from(*source),
                detail: str_of(body, field).unwrap_or("").to_owned(),
            };
        }
    }
    if let Some(body) = volume.get("emptyDir") {
        let medium = str_of(body, "medium").unwrap_or("");
        let size = str_of(body, "sizeLimit").map(|s| format!("limit {s}"));
        let detail = [(!medium.is_empty()).then(|| medium.to_owned()), size]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
        return VolumeSummary {
            name,
            source: Arc::from("EmptyDir"),
            detail,
        };
    }
    if let Some(body) = volume.get("projected") {
        let sources = arr_of(body, "sources").len();
        return VolumeSummary {
            name,
            source: Arc::from("Projected"),
            detail: format!("{sources} source{}", if sources == 1 { "" } else { "s" }),
        };
    }
    if let Some(body) = volume.get("ephemeral") {
        let class = str_of(
            sub(sub(body, "volumeClaimTemplate"), "spec"),
            "storageClassName",
        );
        return VolumeSummary {
            name,
            source: Arc::from("Ephemeral"),
            detail: class.unwrap_or("").to_owned(),
        };
    }
    // Any other source: its key, capitalised.
    let other = volume
        .as_object()
        .and_then(|o| o.keys().find(|k| *k != "name"))
        .unwrap_or("unknown");
    VolumeSummary {
        name,
        source: Arc::from(capitalised(other)),
        detail: String::new(),
    }
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
