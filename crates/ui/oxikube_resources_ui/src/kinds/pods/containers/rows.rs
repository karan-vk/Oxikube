//! [`ContainerRow`]: one row of the containers sub-table as display text and tones, built once
//! per pod version from the domain's [`ContainerSummary`] (never in render).

use gpui::SharedString;
use oxikube_app::columns::{Tone, status_tone};
use oxikube_domain::view::{ContainerState, ResourceAmounts};
use oxikube_domain::{ContainerSummary, Quantity, Resource};

/// What a missing value reads (no request, no limit, no port).
const ABSENT: &str = "–";

/// The sub-table's columns: id (the cell's test selector), title, width in unscaled pixels,
/// right-aligned. k9s' containers view, in its order: IDX, NAME, IMAGE, READY, STATE, RESTARTS,
/// PROBES (L:R:S), PORTS, then CPU and memory as `request/limit`.
pub(super) const COLUMNS: &[(&str, &str, f32, bool)] = &[
    ("idx", "#", 28., true),
    ("name", "Name", 150., false),
    ("image", "Image", 200., false),
    ("ready", "Ready", 52., false),
    ("state", "State", 130., false),
    ("restarts", "Restarts", 64., true),
    ("probes", "Probes L:R:S", 100., false),
    ("ports", "Ports", 140., false),
    ("cpu", "CPU R/L", 100., true),
    ("memory", "Mem R/L", 110., true),
];

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
}

impl ContainerRow {
    /// The texts of the cells, in [`COLUMNS`] order (the name without its marker).
    pub fn cells(&self) -> [SharedString; 10] {
        [
            self.index.to_string().into(),
            self.name.clone(),
            self.image.clone(),
            self.ready.into(),
            self.state.clone(),
            self.restarts.clone(),
            self.probes.into(),
            self.ports.clone(),
            self.cpu.clone(),
            self.memory.clone(),
        ]
    }
}

/// The rows of `pod`'s containers; empty for an object that is not a pod.
pub fn rows_of(pod: &Resource) -> Vec<ContainerRow> {
    let Ok(containers) = ContainerSummary::list_from_resource(pod) else {
        return Vec::new();
    };
    containers
        .iter()
        .enumerate()
        .map(|(index, c)| row(index, c))
        .collect()
}

fn row(index: usize, c: &ContainerSummary) -> ContainerRow {
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
