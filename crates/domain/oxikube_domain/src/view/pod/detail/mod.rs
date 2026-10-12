//! [`PodDetail`]: what the pod detail panel adds to the containers table (E12-S03): each
//! container's environment, mounts and probes, the pod's volumes, tolerations and affinity, and
//! its node and owners.
//!
//! The view-model is built from the pod's JSON alone, once per version, and does no lookups. The
//! environment is the one part that needs other objects: [`PodDetail::source_refs`] lists the
//! ConfigMaps and Secrets the containers name, the caller reads each once, and
//! [`ContainerDetail::env_rows`] resolves the variables against what it found.
//!
//! | File | Holds |
//! |---|---|
//! | `env` | `env` and `envFrom` as the spec writes them: [`EnvVar`], [`EnvFrom`], [`SourceKind`] |
//! | `fields` | the values of `fieldRef` and `resourceFieldRef` |
//! | `resolve` | [`EnvSources`] (what the lookups found) and [`EnvRow`], the resolved variables |
//! | `secret` | [`SecretValue`]: a decoded value that never prints, serialises or logs |
//! | `volumes` | [`VolumeSummary`] and [`MountSummary`] |
//! | `probes` | [`ProbeDetail`] |
//! | `scheduling` | [`Toleration`] and [`AffinityLine`] |

mod env;
mod fields;
mod probes;
mod resolve;
mod scheduling;
mod secret;
mod volumes;

use std::sync::Arc;

pub use env::{EnvFrom, EnvSource, EnvVar, SourceKind};
pub use probes::{ProbeDetail, ProbeKind};
pub use resolve::{
    EnvRow, EnvSources, EnvValue, Missing, Origin, SourceData, SourceState, SourceValue,
};
pub use scheduling::{AffinityLine, Toleration};
pub use secret::{MASK, SecretValue};
pub use volumes::{MountSummary, VolumeSummary};

use super::{ContainerKind, is_restartable};
use crate::resource::{OwnerRef, Resource};
use crate::view::{ViewError, arc_of, arr_of, check_kind, sub};

/// One container's detail: the parts of its spec the containers table has no column for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerDetail {
    /// The container's name.
    pub name: Arc<str>,
    /// Init, sidecar, regular or ephemeral.
    pub kind: ContainerKind,
    /// `env`, in order.
    pub env: Vec<EnvVar>,
    /// `envFrom`, in order.
    pub env_from: Vec<EnvFrom>,
    /// `volumeMounts`.
    pub mounts: Vec<MountSummary>,
    /// The probes the spec defines.
    pub probes: Vec<ProbeDetail>,
}

impl ContainerDetail {
    /// The container's environment against what the lookups found: every `envFrom` key, then the
    /// `env` entries, a later definition of a name replacing an earlier one. `pod` supplies the
    /// `fieldRef` and `resourceFieldRef` values.
    pub fn env_rows(&self, pod: &Resource, sources: &EnvSources) -> Vec<EnvRow> {
        resolve::rows(&self.env, &self.env_from, pod, &self.name, sources)
    }

    /// Whether the container has an environment to show.
    pub fn has_env(&self) -> bool {
        !self.env.is_empty() || !self.env_from.is_empty()
    }
}

/// The detail of one pod: its containers' environment, mounts and probes, its volumes,
/// tolerations and affinity, and its node and owners, built from the pod's JSON alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodDetail {
    /// Every container (init and sidecar first, then regular, then ephemeral).
    pub containers: Vec<ContainerDetail>,
    /// `spec.volumes`.
    pub volumes: Vec<VolumeSummary>,
    /// `spec.tolerations`.
    pub tolerations: Vec<Toleration>,
    /// Node selector, affinity and topology spread, one line each.
    pub affinity: Vec<AffinityLine>,
    /// `spec.nodeName`: the node link.
    pub node: Option<Arc<str>>,
    /// `metadata.ownerReferences`; the controller is the one the owner chain follows.
    pub owners: Vec<OwnerRef>,
}

impl PodDetail {
    /// The detail of a Pod.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `pod` is not a core `Pod`. Missing or malformed fields never
    /// fail.
    pub fn from_resource(pod: &Resource) -> Result<Self, ViewError> {
        check_kind(pod, "Pod", &[("", "Pod")])?;
        let spec = sub(pod.json(), "spec");
        let groups = [
            ("initContainers", ContainerKind::Init),
            ("containers", ContainerKind::Regular),
            ("ephemeralContainers", ContainerKind::Ephemeral),
        ];
        let mut containers = Vec::new();
        for (key, kind) in groups {
            for c in arr_of(spec, key) {
                let Some(name) = arc_of(c, "name") else {
                    continue;
                };
                let kind = if kind == ContainerKind::Init && is_restartable(c) {
                    ContainerKind::Sidecar
                } else {
                    kind
                };
                containers.push(ContainerDetail {
                    name,
                    kind,
                    env: env::env_of(c),
                    env_from: env::env_from_of(c),
                    mounts: volumes::mounts_of(c),
                    probes: probes::probes_of(c),
                });
            }
        }
        Ok(Self {
            containers,
            volumes: volumes::volumes_of(spec),
            tolerations: scheduling::tolerations_of(spec),
            affinity: scheduling::affinity_of(spec),
            node: arc_of(spec, "nodeName"),
            owners: pod.meta.owner_refs.to_vec(),
        })
    }

    /// The ConfigMaps and Secrets the containers' environment names (a key reference or an
    /// `envFrom`), each once, in the order they first appear. The caller looks each up once and
    /// hands the answers to [`ContainerDetail::env_rows`].
    pub fn source_refs(&self) -> Vec<(SourceKind, Arc<str>)> {
        let mut out: Vec<(SourceKind, Arc<str>)> = Vec::new();
        for container in &self.containers {
            let keyed = container.env.iter().filter_map(|var| match &var.source {
                EnvSource::KeyRef { kind, name, .. } => Some((*kind, name.clone())),
                _ => None,
            });
            let from = container.env_from.iter().map(|f| (f.kind, f.name.clone()));
            for source in keyed.chain(from) {
                if !out.contains(&source) {
                    out.push(source);
                }
            }
        }
        out
    }

    /// The controller among the owners.
    pub fn controller(&self) -> Option<&OwnerRef> {
        self.owners.iter().find(|o| o.controller)
    }
}

/// `spec.nodeName` of a pod, without building the whole detail.
pub fn node_of(pod: &Resource) -> Option<Arc<str>> {
    arc_of(sub(pod.json(), "spec"), "nodeName")
}
