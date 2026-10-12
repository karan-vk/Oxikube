//! Pods (E12-S02): the first kind with a module of its own. Its [`KindSpec`] brings
//!
//! * the columns ([`PodColumns`], `oxikube_app`): Name, Namespace, Ready, Status (kubectl v1.36's
//!   printer), Restarts with the last restart, CPU and Memory (E13's metrics, `–` without them),
//!   `%CPU/R`, `%CPU/L`, `%MEM/R`, `%MEM/L`, IP, Node, QoS, Service Account, Controlled By, Age;
//!   IP, Node and QoS hidden by default as in Freelens. The column picker saves what the user
//!   shows, per kind (`ColumnPrefs`);
//! * the containers drill-down ([`ContainersPanel`]) at the top of a pod's detail;
//! * the pod actions E08 and E09 ship (logs, shell, attach, debug, port forward).
//!
//! A user reaches it from the cluster sidebar: Workloads > Pods opens the table; Enter (or a
//! click) on a row opens the detail drawer with the containers.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | the spec and its registration, [`set_metrics_source`] |
//! | `containers` | [`ContainersPanel`], its rows and their drawing |
//! | `panel` | [`PodPanel`] (E12-S03): the action bar, the containers table and the sections (owner chain, node, environment, mounts, probes, volumes, tolerations, affinity) |
//! | `actions` | the row actions of E12-S03: Evict, Delete, Force delete, Sanitize, Jump to owner, Show node |

pub mod actions;
pub mod containers;
pub mod panel;

use std::sync::Arc;

use gpui::{App, Global};
use oxikube_app::columns::MetricsSource;
use oxikube_app::{KindMeta, PodColumns, Section};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, Scope};

pub use actions::row_actions;
pub use containers::ContainersPanel;
pub use panel::PodPanel;

use super::{DetailRenderer, KindSpec, KindSpecs};

/// Pods' slot in the Workloads section: first (the other workloads start at 20).
pub const RANK: u32 = 10;

/// A pod's own actions: logs, shell, attach, debug and port forward as E08 and E09 ship them,
/// and E12-S03's jump to owner, show node, evict, delete, force delete and sanitize.
const POD_ACTIONS: &[CommandId] = &[
    CommandId::POD_VIEW_LOGS,
    CommandId::POD_SHELL,
    CommandId::POD_ATTACH,
    CommandId::POD_DEBUG,
    CommandId::POD_PORT_FORWARD,
    CommandId::POD_JUMP_TO_OWNER,
    CommandId::POD_SHOW_NODE,
    CommandId::POD_EVICT,
    CommandId::POD_DELETE,
    CommandId::POD_FORCE_DELETE,
    CommandId::POD_SANITIZE,
];

/// The registered pod columns, so E13 can hand them its metrics source.
struct PodKind {
    columns: Arc<PodColumns>,
}

impl Global for PodKind {}

/// The Pod spec over `columns`.
pub fn spec(columns: Arc<PodColumns>) -> KindSpec {
    let meta = KindMeta::new(Gvk::new("", "v1", "Pod"), "pods", "Pods", Scope::Namespaced)
        .in_section(Section::WORKLOADS, RANK)
        .with_actions(POD_ACTIONS.iter().copied())
        .with_columns(columns);
    KindSpec::new(meta).with_detail(DetailRenderer::panel(PodPanel::build))
}

/// Registers the Pod kind (once; a second call does nothing).
pub fn init(cx: &mut App) {
    if cx.has_global::<PodKind>() {
        return;
    }
    let columns = Arc::new(PodColumns::new());
    cx.set_global(PodKind {
        columns: columns.clone(),
    });
    if let Err(error) = KindSpecs::register(cx, spec(columns)) {
        tracing::error!(%error, "the Pod kind was not registered");
    }
}

/// Gives the pods table its CPU and memory source (E13), or takes it away with `None`. Without
/// one the usage and `%` columns read `–`.
pub fn set_metrics_source(cx: &App, source: Option<Arc<dyn MetricsSource>>) {
    if let Some(kind) = cx.try_global::<PodKind>() {
        kind.columns.set_metrics(source);
    }
}

/// The pods table's column provider, once [`init`] ran.
pub fn columns(cx: &App) -> Option<Arc<PodColumns>> {
    cx.try_global::<PodKind>().map(|kind| kind.columns.clone())
}

#[cfg(test)]
mod tests;
