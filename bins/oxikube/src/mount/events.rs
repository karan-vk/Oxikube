//! The events view of the main window (E14-S03): `events::Open` opens a cluster's events view in
//! its tab, and the `events::*` commands act on it.
//!
//! * [`start_views`] applies each `events::*` request on the UI thread
//!   (`oxikube_events_ui::EventsViews`), over the app's `EventService` (the cluster's one events
//!   feed, which the tab keeps running; see `crate::events`), the state store (the filters saved
//!   per cluster) and the file port (the JSON export).
//! * [`install_tab`] runs for every cluster tab: a click on the sidebar's Events heading is
//!   `events::Open` for the tab's cluster.
//!
//! The `:ev` jump (also `:event`, `:events`; `oxikube_app::search::jump`) and the palette's "Show
//! Events" plan the same command, so a user reaches the view from the sidebar, `:ev` or the
//! palette.

use std::rc::Rc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, WeakEntity, Window};
use oxikube_app::ClusterSession;
use oxikube_domain::command::{Command, CommandId};
use oxikube_events_ui::{EventsRequest, EventsViewDeps, EventsViews, EventsViewsDeps};
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, ClusterTabs};

use super::tabs::TabDeps;

/// Starts the window's events views over `requests` (the bus's `events::*` handlers). Keep the
/// entity for the life of the window.
pub fn start_views(
    view: EventsViewDeps,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<EventsRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<EventsViews> {
    let deps = EventsViewsDeps {
        view,
        host: Rc::new(tabs),
    };
    EventsViews::start(deps, requests, window, cx)
}

/// Routes a click on the sidebar's Events heading to `events::Open` for this tab's cluster.
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
        tracing::warn!(%cluster, "the cluster tab has no sidebar: its Events heading does not navigate");
        return;
    };
    let dispatcher = deps.dispatcher.clone();
    window
        .subscribe(&panel, cx, move |_, event: &SidebarEvent, _, cx| {
            let SidebarEvent::Navigate(SidebarTarget::Command(id)) = event else {
                return;
            };
            if *id == CommandId::EVENTS_OPEN {
                let cluster = cluster.clone();
                dispatcher.dispatch(
                    Command::EventsOpen {
                        cluster,
                        namespace: None,
                        filter: None,
                    },
                    cx,
                );
            }
        })
        .detach();
}
