//! The table commands on the `CommandBus`: `resource::Open`, `resource::CopyName`,
//! `resource::RetryFeed` (E07-S10) and `resource::SelectAll`.
//!
//! None changes a cluster (no `MutationGuard` tier): they tell a view to show a detail, select
//! rows, restart a feed (a read) or write the user's clipboard. Each is declared in
//! `oxikube_domain::command`, so it has an MCP tool stub, and [`register_commands`] installs
//! handlers that push a [`ViewRequest`] into the window's [`ResourceCommandSink`];
//! [`ResourceViews`] applies it on the UI thread.
//!
//! `resource::OpenList` is [`navigate`](crate::navigate)'s (E07-S11): its handler hands the
//! request to the window, which asks the registered kind views; [`ResourceViews`] is one of them
//! and opens the table.
//!
//! [`ResourceViews`]: super::ResourceViews

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};

/// The commands this crate handles.
pub const RESOURCE_COMMANDS: [CommandId; 4] = [
    CommandId::RESOURCE_OPEN,
    CommandId::RESOURCE_COPY_NAME,
    CommandId::RESOURCE_RETRY_FEED,
    CommandId::RESOURCE_SELECT_ALL,
];

/// A resource command, resolved, for the UI thread.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewRequest {
    /// Show the detail of `target` (the tables of its kind emit `OpenDetail`).
    Open(ResourceRef),
    /// Copy `target`'s name to the clipboard.
    CopyName(ResourceRef),
    /// Restart the feed of the tables of `gvk` in `cluster`.
    RetryFeed {
        /// The cluster.
        cluster: ClusterId,
        /// The kind.
        gvk: Gvk,
    },
    /// Select every row of the tables of `gvk` in `cluster`.
    SelectAll {
        /// The cluster.
        cluster: ClusterId,
        /// The kind.
        gvk: Gvk,
    },
}

/// A handle on a window's request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct ResourceCommandSink {
    tx: UnboundedSender<ViewRequest>,
}

impl ResourceCommandSink {
    /// A sink and the receiver [`ResourceViews::start`](super::ResourceViews::start) drains.
    pub fn channel() -> (Self, UnboundedReceiver<ViewRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Queues `request`. `false` when the window is gone.
    pub fn send(&self, request: ViewRequest) -> bool {
        self.tx.unbounded_send(request).is_ok()
    }

    /// The request a table command stands for (`None` for any other command).
    pub(crate) fn request_for(command: &Command) -> Option<ViewRequest> {
        Some(match command {
            Command::ResourceOpen { target } => ViewRequest::Open(target.clone()),
            Command::ResourceCopyName { target } => ViewRequest::CopyName(target.clone()),
            Command::ResourceRetryFeed { cluster, gvk } => ViewRequest::RetryFeed {
                cluster: cluster.clone(),
                gvk: gvk.clone(),
            },
            Command::ResourceSelectAll { cluster, gvk } => ViewRequest::SelectAll {
                cluster: cluster.clone(),
                gvk: gvk.clone(),
            },
            _ => return None,
        })
    }
}

/// Registers [`RESOURCE_COMMANDS`] on `registry` with handlers that push into `sink`. Call it
/// from the binary's command setup:
/// `registry.install("oxikube_resources_ui", |r| register_commands(r, sink))`.
///
/// # Errors
///
/// The first [`RegisterError`] (a duplicate registration is a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: ResourceCommandSink,
) -> Result<(), RegisterError> {
    for id in RESOURCE_COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let request = ResourceCommandSink::request_for(&command)
                    .ok_or_else(|| OxiError::validation("not a resource table command"))?;
                if sink.send(request) {
                    Ok(CommandOutput::none())
                } else {
                    Err(OxiError::internal(
                        "the window with the resource views is gone",
                    ))
                }
            }
        })?;
    }
    Ok(())
}
