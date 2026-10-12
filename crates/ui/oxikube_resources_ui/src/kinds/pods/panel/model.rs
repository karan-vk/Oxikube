//! [`Section`]s of the pod panel as plain data, built from the domain's [`PodDetail`] (no window,
//! no clock): the owner chain, the node, and the collapsed sections for the environment, mounts,
//! probes, volumes, tolerations and affinity.

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::view::{ContainerDetail, EnvSources, PodDetail};

use super::env_rows::{EnvCell, env_row};
use super::section::SectionId;

/// The most rows one section draws; past it a note says how many are left. A pod with a huge
/// `envFrom` ConfigMap stays one bounded list.
pub(super) const MAX_ROWS: usize = 300;

/// Which variable of which container a revealed value is: the container's position, the name.
pub type EnvKey = (usize, String);

/// One row of a section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A label and a value.
    Pair {
        /// The label.
        label: String,
        /// The value.
        value: String,
    },
    /// A label and a value that opens another object.
    Link {
        /// The label.
        label: String,
        /// The link text.
        text: String,
        /// What it opens.
        target: ResourceRef,
    },
    /// An environment variable.
    Env(EnvLine),
    /// A sub-heading: a container's name.
    Head(String),
    /// Cells of a mount or a probe: name, then details.
    Cells(Vec<String>),
    /// Muted text.
    Note(String),
}

/// One environment variable of a container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvLine {
    /// Which variable, for the reveal toggle.
    pub key: EnvKey,
    /// Its name.
    pub name: String,
    /// Its value.
    pub cell: EnvCell,
    /// Where it comes from, muted (`ConfigMap web-config`, `fieldRef status.podIP`).
    pub origin: String,
}

/// A titled list of rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// Which section.
    pub id: SectionId,
    /// How many items it lists (rows past `MAX_ROWS` (300) are cut, the count is not).
    pub count: usize,
    /// The rows (empty while the section is collapsed and nothing needs them).
    pub rows: Vec<Row>,
}

/// What the async reads of the panel have found so far.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    /// The pod's owners, nearest first, once read.
    pub chain: Option<Vec<ResourceRef>>,
    /// Why the owner chain could not be read.
    pub chain_error: Option<String>,
    /// The ConfigMaps and Secrets of the environment, once read.
    pub env: Option<EnvSources>,
    /// Whether the environment read is in flight.
    pub env_loading: bool,
    /// Why the environment could not be read.
    pub env_error: Option<String>,
}

/// The panel's data for one version of one pod.
#[derive(Clone, Debug)]
pub struct PodModel {
    /// The pod's detail.
    pub detail: PodDetail,
    /// The pod itself, for field references.
    pub pod: Arc<Resource>,
}

impl PodModel {
    /// The model of `pod`, `None` for an object that is not a pod.
    pub fn new(pod: &Resource) -> Option<Self> {
        Some(Self {
            detail: PodDetail::from_resource(pod).ok()?,
            pod: Arc::new(pod.clone()),
        })
    }

    /// The sections, with rows for the ones in `open`. `target` is the pod's own reference (its
    /// cluster and namespace place the links).
    pub fn sections(
        &self,
        target: &ResourceRef,
        open: &dyn Fn(SectionId) -> bool,
        loaded: &Loaded,
    ) -> Vec<Section> {
        SectionId::ALL
            .into_iter()
            .map(|id| {
                let rows = if open(id) {
                    self.rows(id, target, loaded)
                } else {
                    Vec::new()
                };
                Section {
                    id,
                    count: self.count(id, loaded),
                    rows,
                }
            })
            .collect()
    }

    fn count(&self, id: SectionId, loaded: &Loaded) -> usize {
        let d = &self.detail;
        match id {
            SectionId::Owners => loaded
                .chain
                .as_ref()
                .map_or(d.owners.len().min(1), Vec::len),
            SectionId::Node => usize::from(d.node.is_some()),
            SectionId::Environment => d
                .containers
                .iter()
                .map(|c| c.env.len() + c.env_from.len())
                .sum(),
            SectionId::Mounts => d.containers.iter().map(|c| c.mounts.len()).sum(),
            SectionId::Probes => d.containers.iter().map(|c| c.probes.len()).sum(),
            SectionId::Volumes => d.volumes.len(),
            SectionId::Tolerations => d.tolerations.len(),
            SectionId::Affinity => d.affinity.len(),
        }
    }

    fn rows(&self, id: SectionId, target: &ResourceRef, loaded: &Loaded) -> Vec<Row> {
        let d = &self.detail;
        let mut rows = match id {
            SectionId::Owners => owner_rows(d, loaded),
            SectionId::Node => d
                .node
                .iter()
                .map(|node| Row::Link {
                    label: "Node".into(),
                    text: node.to_string(),
                    target: ResourceRef::cluster_scoped(
                        target.cluster.clone(),
                        Gvk::new("", "v1", "Node"),
                        node.clone(),
                    ),
                })
                .collect(),
            SectionId::Environment => self.env_rows(loaded),
            SectionId::Mounts => per_container(&d.containers, |c| {
                c.mounts
                    .iter()
                    .map(|m| Row::Cells(vec![m.path.to_string(), m.volume.to_string(), m.flags()]))
                    .collect()
            }),
            SectionId::Probes => per_container(&d.containers, |c| {
                c.probes
                    .iter()
                    .map(|p| {
                        Row::Cells(vec![
                            p.kind.as_str().to_owned(),
                            p.action.clone(),
                            p.timing.clone(),
                        ])
                    })
                    .collect()
            }),
            SectionId::Volumes => d
                .volumes
                .iter()
                .map(|v| {
                    Row::Cells(vec![
                        v.name.to_string(),
                        v.source.to_string(),
                        v.detail.clone(),
                    ])
                })
                .collect(),
            SectionId::Tolerations => d
                .tolerations
                .iter()
                .map(|t| Row::Pair {
                    label: "Tolerates".into(),
                    value: t.display(),
                })
                .collect(),
            SectionId::Affinity => d
                .affinity
                .iter()
                .map(|line| Row::Pair {
                    label: line.kind.into(),
                    value: if line.strength.is_empty() {
                        line.text.clone()
                    } else {
                        format!("{}: {}", line.strength, line.text)
                    },
                })
                .collect(),
        };
        if rows.is_empty() {
            rows.push(Row::Note(empty_note(id)));
        }
        if rows.len() > MAX_ROWS {
            let more = rows.len() - MAX_ROWS;
            rows.truncate(MAX_ROWS);
            rows.push(Row::Note(format!("… and {more} more")));
        }
        rows
    }

    fn env_rows(&self, loaded: &Loaded) -> Vec<Row> {
        let containers: Vec<(usize, &ContainerDetail)> = self
            .detail
            .containers
            .iter()
            .enumerate()
            .filter(|(_, c)| c.has_env())
            .collect();
        if containers.is_empty() {
            return Vec::new();
        }
        let Some(sources) = &loaded.env else {
            return vec![Row::Note(match (&loaded.env_error, loaded.env_loading) {
                (Some(error), _) => format!("Could not read the environment's sources: {error}"),
                (None, true) => "Reading the ConfigMaps and Secrets this environment names…".into(),
                (None, false) => "The environment has not been read yet.".into(),
            })];
        };
        let many = containers.len() > 1;
        let mut rows = Vec::new();
        for (ix, container) in containers {
            if many {
                rows.push(Row::Head(container.name.to_string()));
            }
            for row in container.env_rows(&self.pod, sources) {
                rows.push(env_row(ix, row));
            }
        }
        rows
    }
}

fn per_container(
    containers: &[ContainerDetail],
    rows_of: impl Fn(&ContainerDetail) -> Vec<Row>,
) -> Vec<Row> {
    let with: Vec<(&ContainerDetail, Vec<Row>)> = containers
        .iter()
        .map(|c| (c, rows_of(c)))
        .filter(|(_, rows)| !rows.is_empty())
        .collect();
    let many = containers.len() > 1;
    let mut out = Vec::new();
    for (container, rows) in with {
        if many {
            out.push(Row::Head(container.name.to_string()));
        }
        out.extend(rows);
    }
    out
}

fn owner_rows(detail: &PodDetail, loaded: &Loaded) -> Vec<Row> {
    if let Some(chain) = &loaded.chain {
        return chain
            .iter()
            .enumerate()
            .map(|(level, owner)| Row::Link {
                label: if level == 0 {
                    "Owned by"
                } else {
                    "…owned by"
                }
                .into(),
                text: format!("{} {}", owner.gvk.kind, owner.name),
                target: owner.clone(),
            })
            .collect();
    }
    // Until the chain is read, say what the pod's own metadata says: no link, the scope of the
    // owner is not known yet.
    let mut rows: Vec<Row> = detail
        .controller()
        .or_else(|| detail.owners.first())
        .map(|owner| Row::Pair {
            label: "Owned by".into(),
            value: format!("{} {}", owner.kind, owner.name),
        })
        .into_iter()
        .collect();
    if let Some(error) = &loaded.chain_error {
        rows.push(Row::Note(format!("The owners could not be read: {error}")));
    }
    rows
}

fn empty_note(id: SectionId) -> String {
    match id {
        SectionId::Owners => "Nobody owns this pod.",
        SectionId::Node => "Not scheduled on a node yet.",
        SectionId::Environment => "No environment variables.",
        SectionId::Mounts => "No volume mounts.",
        SectionId::Probes => "No probes.",
        SectionId::Volumes => "No volumes.",
        SectionId::Tolerations => "No tolerations.",
        SectionId::Affinity => "No node selector or affinity.",
    }
    .to_owned()
}
