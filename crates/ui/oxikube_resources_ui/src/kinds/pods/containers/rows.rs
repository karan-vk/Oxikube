//! [`ContainerRow`]: one row of the containers sub-table as display text and tones, built once
//! per pod version and once per metrics poll from the domain's [`ContainerSummary`] and the
//! container's reading (never in render).

use gpui::SharedString;
use oxikube_app::ClusterUsage;
use oxikube_app::columns::pods::ABSENT;
use oxikube_app::columns::{Tone, UsageThresholds, status_tone};
use oxikube_app::metrics::live::ContainerUsageCells;
use oxikube_domain::metrics::ContainerUsage;
use oxikube_domain::view::{ContainerState, ResourceAmounts};
use oxikube_domain::{ContainerSummary, Quantity, Resource};

/// The sub-table's columns: id (the cell's test selector), title, width in unscaled pixels,
/// right-aligned. k9s' containers view, in its order: IDX, NAME, IMAGE, READY, STATE, RESTARTS,
/// PROBES (L:R:S), usage (CPU, MEM, E13-S03), requests/limits (`request/limit`), the usage over
/// them (`%CPU/R` ... `%MEM/L`, the `/L` ones toned by `metrics.thresholds`), PORTS.
pub(super) const COLUMNS: &[(&str, &str, f32, bool)] = &[
    ("idx", "#", 28., true),
    ("name", "Name", 150., false),
    ("image", "Image", 200., false),
    ("ready", "Ready", 52., false),
    ("state", "State", 130., false),
    ("restarts", "Restarts", 64., true),
    ("probes", "Probes L:R:S", 100., false),
    ("cpu-usage", "CPU", 60., true),
    ("memory-usage", "MEM", 64., true),
    ("cpu", "CPU R/L", 100., true),
    ("memory", "Mem R/L", 110., true),
    ("cpu-request-pct", "%CPU/R", 60., true),
    ("cpu-limit-pct", "%CPU/L", 60., true),
    ("memory-request-pct", "%MEM/R", 60., true),
    ("memory-limit-pct", "%MEM/L", 60., true),
    ("ports", "Ports", 140., false),
];

/// How many columns there are.
const WIDTH: usize = 16;

/// One container, ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerRow {
    /// The container's position in the pod (init containers first, as listed).
    pub index: usize,
    /// The container name.
    pub name: SharedString,
    /// `init`, `sidecar` or `ephemeral`; empty for a regular container.
    pub marker: &'static str,
    /// The image, `–` when unknown.
    pub image: SharedString,
    /// `true` / `false`.
    pub ready: &'static str,
    /// The state's reason or name (`Running`, `CrashLoopBackOff`, `Completed`...).
    pub state: SharedString,
    /// The state's tone (the table's status rules).
    pub state_tone: Tone,
    /// The restart count.
    pub restarts: SharedString,
    /// `on:off:on`: liveness, readiness, startup.
    pub probes: &'static str,
    /// `http:8080/TCP, 9090/TCP`, `–` without ports.
    pub ports: SharedString,
    /// CPU `request/limit` (`100m/500m`, `100m/–`).
    pub cpu: SharedString,
    /// Memory `request/limit` (`128Mi/256Mi`).
    pub memory: SharedString,
    /// Usage from metrics-server and its share of the request and limit; `–` without a reading.
    pub usage: ContainerUsageCells,
}

impl ContainerRow {
    /// The texts of the cells, in `COLUMNS` order (the name without its marker).
    pub fn cells(&self) -> [SharedString; WIDTH] {
        let text =
            |cell: &oxikube_app::Cell<'static>| SharedString::from(cell.display().to_owned());
        let u = &self.usage;
        [
            self.index.to_string().into(),
            self.name.clone(),
            self.image.clone(),
            self.ready.into(),
            self.state.clone(),
            self.restarts.clone(),
            self.probes.into(),
            text(&u.cpu),
            text(&u.memory),
            self.cpu.clone(),
            self.memory.clone(),
            text(&u.cpu_of_request),
            text(&u.cpu_of_limit),
            text(&u.memory_of_request),
            text(&u.memory_of_limit),
            self.ports.clone(),
        ]
    }

    /// The tone of column `id` (the usage `%` cells' thresholds), `None` for a column drawn in
    /// its own colour.
    pub fn usage_tone(&self, id: &str) -> Option<Tone> {
        let u = &self.usage;
        Some(match id {
            "cpu-limit-pct" => u.cpu_of_limit.tone(),
            "memory-limit-pct" => u.memory_of_limit.tone(),
            _ => return None,
        })
    }
}

/// The rows of `pod`'s containers with their readings from `usage` (the pod's cluster); empty
/// for an object that is not a pod.
pub fn rows_of(pod: &Resource, usage: Option<&ClusterUsage>) -> Vec<ContainerRow> {
    let Ok(containers) = ContainerSummary::list_from_resource(pod) else {
        return Vec::new();
    };
    let readings = match (usage, pod.meta.namespace.as_deref()) {
        (Some(usage), Some(namespace)) => usage.containers(namespace, &pod.meta.name),
        _ => Vec::new(),
    };
    let thresholds = usage.map(ClusterUsage::thresholds).unwrap_or_default();
    containers
        .iter()
        .enumerate()
        .map(|(index, c)| row(index, c, &readings, &thresholds))
        .collect()
}

fn row(
    index: usize,
    c: &ContainerSummary,
    readings: &[ContainerUsage],
    thresholds: &UsageThresholds,
) -> ContainerRow {
    let state = c.state.label().to_owned();
    let state_tone = match &c.state {
        // A container that is up but not ready reads amber, as the pod's Ready column does.
        ContainerState::Running { .. } if !c.ready => Tone::Warn,
        _ => status_tone(&state),
    };
    let ports = if c.ports.is_empty() {
        ABSENT.to_owned()
    } else {
        c.ports
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    ContainerRow {
        index,
        name: c.name.to_string().into(),
        marker: c.kind.marker(),
        image: c.image.as_deref().unwrap_or(ABSENT).to_owned().into(),
        ready: if c.ready { "true" } else { "false" },
        state: state.into(),
        state_tone,
        restarts: c.restarts.to_string().into(),
        probes: c.probes.display(),
        ports: ports.into(),
        cpu: pair(&c.resources.requests, &c.resources.limits, |a| a.cpu),
        memory: pair(&c.resources.requests, &c.resources.limits, |a| a.memory),
        usage: ContainerUsageCells::new(
            readings.iter().find(|r| *r.name == *c.name),
            &c.resources.requests,
            &c.resources.limits,
            thresholds,
        ),
    }
}

/// `request/limit`, each as written in the spec (`–` when unset).
fn pair(
    requests: &ResourceAmounts,
    limits: &ResourceAmounts,
    pick: fn(&ResourceAmounts) -> Option<Quantity>,
) -> SharedString {
    let show = |q: Option<Quantity>| q.map_or_else(|| ABSENT.to_owned(), |q| q.to_string());
    format!("{}/{}", show(pick(requests)), show(pick(limits))).into()
}
