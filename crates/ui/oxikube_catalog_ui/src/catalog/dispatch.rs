//! [`CommandDispatcher`] (defined in `oxikube_workspace` so the cluster tabs and the hotbar share
//! it) and [`ServiceDispatcher`]: how the catalog sends the commands its keys and clicks stand
//! for.

use gpui::App;
use oxikube_app::{ClusterCommandOutcome, ClusterCommands};
use oxikube_domain::command::Command;
use oxikube_runtime::spawn_kube;

pub use oxikube_workspace::CommandDispatcher;

/// Runs the cluster commands against [`ClusterCommands`] on the Tokio bridge.
///
/// Connecting reads from the cluster and never mutates it, so none of these commands goes
/// through `MutationGuard`. A command it does not own is logged and dropped.
#[derive(Clone)]
pub struct ServiceDispatcher {
    commands: ClusterCommands,
}

impl ServiceDispatcher {
    /// A dispatcher over `commands`.
    pub fn new(commands: ClusterCommands) -> Self {
        Self { commands }
    }
}

impl CommandDispatcher for ServiceDispatcher {
    fn dispatch(&self, command: Command, cx: &mut App) {
        if !ClusterCommands::handles(&command) {
            tracing::warn!(command = %command.id(), "the catalog dispatcher does not run this command");
            return;
        }
        let commands = self.commands.clone();
        let id = command.id();
        // Detached on purpose: a connect the user asked for runs to its end even when the view
        // that asked is closed, and a later `cluster::Disconnect` cancels it through the
        // session manager. The task owns nothing that is cleared from inside it.
        cx.spawn(async move |cx| {
            let result = spawn_kube(&*cx, async move { commands.handle(&command).await }).await;
            match result {
                Ok(Ok(ClusterCommandOutcome::Connected(state))) => {
                    tracing::debug!(%id, phase = %state.phase(), "cluster command finished");
                }
                Ok(Ok(outcome)) => tracing::debug!(%id, ?outcome, "cluster command finished"),
                Ok(Err(error)) => tracing::warn!(%id, %error, "cluster command failed"),
                Err(error) => tracing::warn!(%id, %error, "cluster command task failed"),
            }
        })
        .detach();
    }
}
