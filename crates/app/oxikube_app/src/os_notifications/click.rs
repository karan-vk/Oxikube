//! [`ClickAction`]: what a click on an OS notification asks the app to do.

use std::sync::Arc;

use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_ports::NotificationActivation;

/// A click on a shown OS notification: bring Oxikube to the front, then show where it was about.
///
/// The UI side does the first (it owns the window); [`commands`](Self::commands) is the second,
/// the same `Command`s a palette entry or a keymap binding would dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClickAction {
    /// The cluster whose tab to show, if the notification concerned one.
    pub cluster: Option<ClusterId>,
    /// The object whose detail to open, if the notification named one.
    pub open: Option<ResourceRef>,
}

impl ClickAction {
    /// The commands to dispatch, in order: `cluster::Select` for the cluster, then
    /// `resource::Open` for the object. Empty for a notification about nothing in particular
    /// (the test notification): the click only focuses the window.
    pub fn commands(&self) -> Vec<Command> {
        let mut commands = Vec::new();
        if let Some(cluster) = self
            .cluster
            .clone()
            .or_else(|| self.open.as_ref().map(|target| target.cluster.clone()))
        {
            commands.push(Command::ClusterSelect { cluster });
        }
        if let Some(target) = self.open.clone() {
            commands.push(Command::ResourceOpen { target });
        }
        commands
    }
}

impl From<NotificationActivation> for ClickAction {
    fn from(activation: NotificationActivation) -> Self {
        Self {
            cluster: activation.cluster,
            open: activation.action_target,
        }
    }
}

/// Receives the clicks. Called on an adapter thread: forward to the UI and return.
pub type ClickSink = Arc<dyn Fn(ClickAction) + Send + Sync>;
