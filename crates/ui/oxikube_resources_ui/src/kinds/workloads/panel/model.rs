//! [`PanelModel`]: what the workload panel shows of the object, as plain data built once per
//! object version (never in a render): the replica stats, the strategy and selector lines and the
//! pod template's containers.

use std::sync::Arc;

use oxikube_app::columns::Tone;
use oxikube_domain::Resource;
use oxikube_domain::view::{TemplateContainer, WorkloadDetail, WorkloadKind, WorkloadSummary};

/// One number of the replica summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    /// `Ready`, `Up-to-date`, `Available`...
    pub label: &'static str,
    /// `3/3`, `2`.
    pub value: String,
    /// Amber when short of what is wanted.
    pub tone: Tone,
}

/// One container of the pod template, as lines of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerLine {
    /// `app`, or `migrate (init)`.
    pub name: String,
    /// The image reference.
    pub image: Arc<str>,
    /// `http:8080/TCP, 9090/TCP`; empty when none are declared.
    pub ports: String,
    /// `requests cpu 100m, memory 128Mi · limits memory 256Mi`; empty when none are set.
    pub resources: String,
}

/// The panel's data. See the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelModel {
    /// The object version it was built from, to rebuild only when it changes.
    pub version: Option<Arc<str>>,
    /// The object's uid, for the owned objects.
    pub uid: Option<Arc<str>>,
    /// Which controller.
    pub kind: WorkloadKind,
    /// The replica summary.
    pub stats: Vec<Stat>,
    /// `RollingUpdate · Max surge 25% · Max unavailable 25%`; `None` for kinds without one.
    pub strategy: Option<String>,
    /// The pod selector as `kubectl` prints it; empty for "selects nothing special".
    pub selector: String,
    /// Other lines of the spec: `Min ready seconds 5`, `Service db-headless`.
    pub extras: Vec<(&'static str, String)>,
    /// Init containers first, then the containers.
    pub containers: Vec<ContainerLine>,
}

impl PanelModel {
    /// The model of `res`; `None` for a kind the panel does not serve or an object with no
    /// spec yet (a metadata-only read).
    pub fn build(res: &Resource) -> Option<Self> {
        if res.is_partial() {
            return None;
        }
        let summary = WorkloadSummary::from_resource(res).ok()?;
        let detail = WorkloadDetail::from_resource(res).ok()?;
        let mut extras = Vec::new();
        if let Some(service) = &detail.service_name {
            extras.push(("Service", service.to_string()));
        }
        if let Some(seconds) = detail.min_ready_seconds {
            extras.push(("Min ready seconds", seconds.to_string()));
        }
        if let Some(limit) = detail.revision_history_limit {
            extras.push(("Revision history", limit.to_string()));
        }
        if summary.paused {
            extras.push(("Paused", "yes: rollouts wait".to_owned()));
        }
        Some(Self {
            version: res.meta.resource_version.clone(),
            uid: res.meta.uid.clone(),
            kind: summary.kind,
            stats: stats(&summary),
            strategy: detail.strategy.as_ref().map(|s| {
                let mut line = s.kind.to_string();
                for (label, value) in &s.parameters {
                    line.push_str(&format!(" · {label} {value}"));
                }
                line
            }),
            selector: detail.selector.display(),
            extras,
            containers: detail.containers.iter().map(container_line).collect(),
        })
    }
}

fn short(have: u32, want: u32) -> Tone {
    if have >= want { Tone::Ok } else { Tone::Warn }
}

fn stats(s: &WorkloadSummary) -> Vec<Stat> {
    let stat = |label, value: String, tone| Stat { label, value, tone };
    match s.kind {
        WorkloadKind::Deployment => vec![
            stat("Ready", s.ready_display(), short(s.ready, s.desired)),
            stat(
                "Up-to-date",
                s.updated.to_string(),
                short(s.updated, s.desired),
            ),
            stat(
                "Available",
                s.available.to_string(),
                short(s.available, s.desired),
            ),
            stat("Replicas", s.desired.to_string(), Tone::Neutral),
        ],
        WorkloadKind::StatefulSet => {
            let mut stats = vec![
                stat("Ready", s.ready_display(), short(s.ready, s.desired)),
                stat(
                    "Updated",
                    s.updated.to_string(),
                    short(s.updated, s.desired),
                ),
                stat("Replicas", s.desired.to_string(), Tone::Neutral),
            ];
            if let Some(partition) = s.partition.filter(|p| *p > 0) {
                stats.push(stat("Partition", partition.to_string(), Tone::Warn));
            }
            stats
        }
        _ => vec![
            stat("Ready", s.ready_display(), short(s.ready, s.desired)),
            stat(
                "Current",
                s.current.to_string(),
                short(s.current, s.desired),
            ),
            stat("Desired", s.desired.to_string(), Tone::Neutral),
        ],
    }
}

fn pairs(pairs: &[(Arc<str>, Arc<str>)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn container_line(c: &TemplateContainer) -> ContainerLine {
    let mut resources = String::new();
    if !c.requests.is_empty() {
        resources.push_str(&format!("requests {}", pairs(&c.requests)));
    }
    if !c.limits.is_empty() {
        if !resources.is_empty() {
            resources.push_str(" · ");
        }
        resources.push_str(&format!("limits {}", pairs(&c.limits)));
    }
    ContainerLine {
        name: if c.init {
            format!("{} (init)", c.name)
        } else {
            c.name.to_string()
        },
        image: c.image.clone(),
        ports: c.ports.join(", "),
        resources,
    }
}
