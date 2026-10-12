//! The pulses page of the main window (E13-S10): `cluster::ShowPulses` opens it in the cluster's
//! tab, and the sidebar's Pulses entry asks for it.
//!
//! * [`start_views`] applies each `cluster::ShowPulses` request on the UI thread
//!   (`oxikube_overview_ui::pulses::PulsesViews`), over the window's stores, sessions and the
//!   app's live metrics service.
//! * [`install_tab`] runs for every cluster tab: a click on the sidebar's Pulses entry is the
//!   same command, with the tab's cluster.
//!
//! The `:pulses` jump (`oxikube_app::search::jump`) plans the same command.

use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Task, WeakEntity, Window};
use oxikube_app::{ClusterSession, ClusterSessionManager, MetricsService, ResourceStores};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_overview_ui::pulses::{PulsesDeps, PulsesViews, PulsesViewsDeps};
use oxikube_ports::ClockPort;
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, ClusterTabs, CommandDispatcher};

use super::tabs::TabDeps;

/// What the pulses pages are built from.
pub struct PulsesParts {
    /// The sessions.
    pub sessions: ClusterSessionManager,
    /// The per-cluster stores the tiles count from.
    pub stores: Arc<ResourceStores>,
    /// Where a drill-down sends its command (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The live metrics service behind the CPU and memory gauges.
    pub metrics: MetricsService,
    /// The clock the history is timed on.
    pub clock: Arc<dyn ClockPort>,
}

/// Starts the window's pulses pages over `requests` (the bus's `cluster::ShowPulses` handler).
/// Keep the task for the life of the window.
pub fn start_views(
    parts: PulsesParts,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<ClusterId>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let deps = PulsesViewsDeps {
        page: PulsesDeps {
            sessions: parts.sessions,
            stores: parts.stores,
            dispatcher: parts.dispatcher,
            metrics: Some(parts.metrics),
            clock: parts.clock,
        },
        tabs,
    };
    PulsesViews::start(deps, requests, window, cx)
}

/// Routes a click on the sidebar's Pulses entry to `cluster::ShowPulses` for this tab's cluster.
pub fn install_tab(
    tab: &Entity<ClusterTab>,
    session: &ClusterSession,
    deps: &TabDeps,
    window: &mut Window,
    cx: &mut App,
) {
    let cluster = session.id().clone();
    let workspace = tab.read(cx).workspace().clone();
    let Some(panel) = workspace.read(cx).panel::<SidebarPanel>() else {
        tracing::warn!(%cluster, "the cluster tab has no sidebar: its Pulses entry does not navigate");
        return;
    };
    let dispatcher = deps.dispatcher.clone();
    window
        .subscribe(&panel, cx, move |_, event: &SidebarEvent, _, cx| {
            let SidebarEvent::Navigate(SidebarTarget::Command(id)) = event else {
                return;
            };
            if *id == CommandId::CLUSTER_SHOW_PULSES {
                let cluster = cluster.clone();
                dispatcher.dispatch(Command::ClusterShowPulses { cluster }, cx);
            }
        })
        .detach();
}
