//! The audit log viewer of the main window (E19-S09): `audit::Open` (the palette's "Audit Log")
//! opens it in the active cluster's tab, `audit::Export` writes its filtered records to a file.
//!
//! [`start_views`] applies each `audit::*` request on the UI thread
//! (`oxikube_resources_ui::audit::AuditViews`) over the guard's own [`AuditLog`] (the signal that
//! records were stored, so a mutation shows up without polling) and the state store (the reads).

use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, WeakEntity, Window};
use oxikube_app::audit::query::AuditReader;
use oxikube_app::{AuditLog, ClusterSessionManager};
use oxikube_ports::{ClockPort, FsPort, StatePort};
use oxikube_resources_ui::audit::{AuditRequest, AuditViewDeps, AuditViews, AuditViewsDeps};
use oxikube_workspace::{ClusterTabs, CommandDispatcher};

/// What the viewer reads and writes.
pub struct AuditWiring {
    /// The guard's audit log: the viewer listens to its `appended` signal.
    pub log: Arc<AuditLog>,
    /// The state store the log is read from.
    pub state: Arc<dyn StatePort>,
    /// The sessions (context names of the cluster cells).
    pub sessions: ClusterSessionManager,
    /// Where the viewer sends its commands (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// Where an export is written.
    pub fs: Arc<dyn FsPort>,
    /// The clock.
    pub clock: Arc<dyn ClockPort>,
}

/// Starts the window's audit views over `requests` (the bus's `audit::*` handlers). Keep the
/// entity for the life of the window.
pub fn start_views(
    wiring: AuditWiring,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<AuditRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<AuditViews> {
    let deps = AuditViewsDeps {
        view: AuditViewDeps {
            reader: AuditReader::new(wiring.state),
            listener: wiring.log.appended(),
            sessions: wiring.sessions,
            dispatcher: wiring.dispatcher,
            fs: wiring.fs,
            clock: wiring.clock,
        },
        host: Rc::new(tabs),
    };
    AuditViews::start(deps, requests, window, cx)
}
