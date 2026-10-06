//! The tab commands: the queue the key handlers and the `CommandBus` push into, and what each
//! command does.
//!
//! `cluster::Select`, `cluster::SwitchTab`, `cluster::NextTab`, `cluster::PreviousTab` and
//! `cluster::CloseTab` change what the window shows, never a cluster, so none goes through
//! `MutationGuard`. Each is declared in `oxikube_domain::command` (so it has an MCP tool stub,
//! `app.cluster_select` and so on) and registered on the bus by [`register_commands`], whose
//! handlers push the command into the controller's queue; the keys (`cmd-1..9`, `ctrl-tab`),
//! the hotbar and the tab's close button reach the same `apply`.

use gpui::{Context, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};

use super::ClusterTabs;
use crate::cluster_tab::dispatch::CommandSink;

/// The commands the tabs run.
const TAB_COMMANDS: [CommandId; 5] = [
    CommandId::CLUSTER_SELECT,
    CommandId::CLUSTER_SWITCH_TAB,
    CommandId::CLUSTER_NEXT_TAB,
    CommandId::CLUSTER_PREVIOUS_TAB,
    CommandId::CLUSTER_CLOSE_TAB,
];

impl ClusterTabs {
    /// Runs a tab command. Returns whether `command` is one of the tab commands and found
    /// something to act on (a cluster with a tab, an index within the open tabs).
    pub fn apply(
        &mut self,
        command: &Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match command {
            Command::ClusterSelect { cluster } => self.activate(cluster, window, cx),
            Command::ClusterSwitchTab { index } => match usize::from(*index).checked_sub(1) {
                Some(index) => self.switch_to_index(index, window, cx),
                None => false,
            },
            Command::ClusterNextTab => self.next(window, cx),
            Command::ClusterPreviousTab => self.previous(window, cx),
            Command::ClusterCloseTab { cluster } => {
                let known = self.tabs.contains_key(cluster);
                self.request_close(cluster, window, cx);
                known
            }
            other => {
                tracing::warn!(command = %other.id(), "not a cluster tab command");
                false
            }
        }
    }
}

/// Registers the tab commands on `registry` (MCP tool stubs included), with handlers that push
/// the command into `sink` ([`ClusterTabs::command_sink`]). Call it from the binary's command
/// setup: `registry.install("oxikube_workspace", |r| register_commands(r, sink))`.
///
/// # Errors
///
/// The first [`RegisterError`] (a duplicate registration is a bug in the binary's setup).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: CommandSink,
) -> Result<(), RegisterError> {
    for id in TAB_COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                if sink.send(command) {
                    Ok(CommandOutput::none())
                } else {
                    Err(OxiError::internal(
                        "the window with the cluster tabs is gone",
                    ))
                }
            }
        })?;
    }
    Ok(())
}
