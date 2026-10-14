//! The events view's commands on the `CommandBus` (E14-S03): `events::Open` (`:ev`, the sidebar's
//! Events heading, the palette's "Show Events") opens a cluster's events view in its tab, and
//! `events::ToggleFaults`, `TogglePause`, `CopyRow`, `CopyRowJson`, `ExportJson` act on it.
//!
//! None changes a cluster, so none has a `MutationGuard` tier and all run on a read-only
//! cluster: they read the app's `EventService` or change what the view shows; the export writes
//! only the file the user picks. Each is declared in `oxikube_domain::command`, so it has an MCP
//! tool stub (`k8s.events_open`, `k8s.events_toggle_faults`, ...). [`register_commands`]
//! installs handlers that push an [`EventsRequest`] into the window's [`EventsCommandSink`];
//! [`EventsViews`] applies it on the UI thread.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | [`EVENTS_COMMANDS`], [`EventsRequest`], [`ViewChange`], [`EventsCommandSink`], [`register_commands`] |
//! | `controller` | [`EventsViews`], [`EventsHost`]: opens the views in their cluster's tab and applies the requests |

mod controller;
#[cfg(test)]
mod tests;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;

pub use controller::{EventsHost, EventsViews, EventsViewsDeps};

/// The commands this crate handles.
pub const EVENTS_COMMANDS: [CommandId; 6] = [
    CommandId::EVENTS_OPEN,
    CommandId::EVENTS_TOGGLE_FAULTS,
    CommandId::EVENTS_TOGGLE_PAUSE,
    CommandId::EVENTS_COPY_ROW,
    CommandId::EVENTS_COPY_ROW_JSON,
    CommandId::EVENTS_EXPORT_JSON,
];

/// What an `events::*` command does to the cluster's events view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewChange {
    /// `events::Open`: show the view, with these filters when given.
    Open {
        /// Only the events stored in this namespace.
        namespace: Option<String>,
        /// The text filter.
        filter: Option<String>,
    },
    /// `events::ToggleFaults`.
    ToggleFaults,
    /// `events::TogglePause`.
    TogglePause,
    /// `events::CopyRow`.
    CopyRow,
    /// `events::CopyRowJson`.
    CopyRowJson,
    /// `events::ExportJson`.
    ExportJson,
}

/// One `events::*` command, for the window's [`EventsViews`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventsRequest {
    /// The cluster whose view it is about.
    pub cluster: ClusterId,
    /// What to do.
    pub change: ViewChange,
}

impl EventsRequest {
    /// The request `command` makes; `None` for a command that is not an `events::*` one.
    pub fn of(command: &Command) -> Option<Self> {
        let (cluster, change) = match command {
            Command::EventsOpen {
                cluster,
                namespace,
                filter,
            } => (
                cluster,
                ViewChange::Open {
                    namespace: namespace.clone(),
                    filter: filter.clone(),
                },
            ),
            Command::EventsToggleFaults { cluster } => (cluster, ViewChange::ToggleFaults),
            Command::EventsTogglePause { cluster } => (cluster, ViewChange::TogglePause),
            Command::EventsCopyRow { cluster } => (cluster, ViewChange::CopyRow),
            Command::EventsCopyRowJson { cluster } => (cluster, ViewChange::CopyRowJson),
            Command::EventsExportJson { cluster } => (cluster, ViewChange::ExportJson),
            _ => return None,
        };
        Some(Self {
            cluster: cluster.clone(),
            change,
        })
    }
}

/// A handle on a window's events request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct EventsCommandSink {
    tx: UnboundedSender<EventsRequest>,
}

impl EventsCommandSink {
    /// A sink and the receiver [`EventsViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<EventsRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Queues `request`. `false` when the window is gone.
    pub fn send(&self, request: EventsRequest) -> bool {
        self.tx.unbounded_send(request).is_ok()
    }
}

/// Registers [`EVENTS_COMMANDS`] on `registry` with handlers that push into `sink`. The binary
/// installs it as `oxikube_events_ui`.
///
/// # Errors
///
/// The first [`RegisterError`] (a duplicate registration is a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: EventsCommandSink,
) -> Result<(), RegisterError> {
    for id in EVENTS_COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let request = EventsRequest::of(&command)
                    .ok_or_else(|| OxiError::validation("not an events command"))?;
                if sink.send(request) {
                    Ok(CommandOutput::none())
                } else {
                    Err(OxiError::internal(
                        "the window with the events views is gone",
                    ))
                }
            }
        })?;
    }
    Ok(())
}
