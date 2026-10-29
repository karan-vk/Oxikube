//! The audit viewer's commands on the `CommandBus` (E19-S09): `audit::Open` and `audit::Export`.
//!
//! Neither changes a cluster, so neither has a `MutationGuard` tier. Each is declared in
//! `oxikube_domain::command`, so it has an MCP tool stub (`app.audit_open`, `app.audit_export`).
//! [`register_commands`] installs handlers that push an [`AuditRequest`] into the window's
//! [`AuditCommandSink`]; [`AuditViews`](super::AuditViews) applies it on the UI thread.

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::audit::AuditExportFormat;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;

/// The commands this module handles.
pub const AUDIT_COMMANDS: [CommandId; 2] = [CommandId::AUDIT_OPEN, CommandId::AUDIT_EXPORT];

/// What an `audit::*` command asks of the viewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuditRequest {
    /// `audit::Open`: show the viewer, filtered to `cluster` when given.
    Open {
        /// The cluster filter to apply.
        cluster: Option<ClusterId>,
    },
    /// `audit::Export`: write the open viewer's filtered records.
    Export {
        /// The file format.
        format: AuditExportFormat,
    },
}

impl AuditRequest {
    /// The request `command` makes; `None` for a command that is not an `audit::*` one.
    pub fn of(command: &Command) -> Option<Self> {
        match command {
            Command::AuditOpen { cluster } => Some(Self::Open {
                cluster: cluster.clone(),
            }),
            Command::AuditExport { format } => Some(Self::Export { format: *format }),
            _ => None,
        }
    }
}

/// A handle on a window's audit request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct AuditCommandSink {
    tx: UnboundedSender<AuditRequest>,
}

impl AuditCommandSink {
    /// A sink and the receiver [`AuditViews::start`](super::AuditViews::start) drains.
    pub fn channel() -> (Self, UnboundedReceiver<AuditRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Queues `request`. `false` when the window is gone.
    pub fn send(&self, request: AuditRequest) -> bool {
        self.tx.unbounded_send(request).is_ok()
    }
}

/// Registers [`AUDIT_COMMANDS`] on `registry` with handlers that push into `sink`. The binary
/// installs it as `oxikube_resources_ui::audit`.
///
/// # Errors
///
/// The first [`RegisterError`] (a duplicate registration is a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: AuditCommandSink,
) -> Result<(), RegisterError> {
    for id in AUDIT_COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let request = AuditRequest::of(&command)
                    .ok_or_else(|| OxiError::validation("not an audit command"))?;
                if sink.send(request) {
                    Ok(CommandOutput::none())
                } else {
                    Err(OxiError::internal(
                        "the window with the audit viewer is gone",
                    ))
                }
            }
        })?;
    }
    Ok(())
}
