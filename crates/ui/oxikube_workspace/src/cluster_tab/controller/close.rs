//! Closing a cluster tab: confirm while the cluster's operations run, then disconnect.
//!
//! Closing a tab disconnects its cluster (a tab exists exactly while its session does). Nothing
//! is lost when nothing runs, so the common case closes without a question. While the cluster
//! has exec sessions or port-forwards (operations registered with
//! [`register_operation_provider`](crate::session::register_operation_provider) against this
//! cluster) the window's modal layer asks first and lists them; "Keep Open" (or Escape) leaves
//! the tab and the session as they were.

use gpui::{AppContext as _, Context, Window};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;

use super::ClusterTabs;
use crate::{
    modal::DialogModal,
    session::{RunningOperation, quit::operation_lines, running_operations},
};

impl ClusterTabs {
    /// The operations running against `cluster` now.
    pub fn running_operations(&self, cluster: &ClusterId, cx: &gpui::App) -> Vec<RunningOperation> {
        running_operations(cx)
            .into_iter()
            .filter(|operation| operation.cluster.as_ref() == Some(cluster))
            .collect()
    }

    /// Closes `cluster`'s tab the way the user asked: straight away when nothing of the
    /// cluster is running, otherwise after the confirmation dialog. Either way it sends
    /// `cluster::Disconnect`, and the tab goes when the session does (a restored placeholder,
    /// which is not connected, just closes). Does nothing for a cluster without a tab.
    pub fn request_close(
        &mut self,
        cluster: &ClusterId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.tabs.get(cluster) else {
            return;
        };
        if self.pending.remove(cluster) {
            // A restored placeholder: nothing is connected, so there is nothing to disconnect or
            // to lose. The session stays open (and disconnected) in the catalog.
            self.close_tab_now(cluster, window, cx);
            return;
        }
        let operations = self.running_operations(cluster, cx);
        if operations.is_empty() {
            self.disconnect(cluster, cx);
            return;
        }
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let title = entry.tab.read(cx).info().title.clone();
        let message = format!(
            "These are still running and will be stopped:\n{}",
            operation_lines(&operations).join("\n")
        );
        let dispatcher = self.deps.dispatcher.clone();
        let cluster = cluster.clone();
        let dialog = cx.new(|cx| {
            DialogModal::new(format!("Close {title}?"), cx)
                .message(message)
                .confirm_label("Disconnect")
                .cancel_label("Keep Open")
                .destructive()
                .on_confirm(move |_, cx| {
                    dispatcher.dispatch(
                        Command::ClusterDisconnect {
                            cluster: cluster.clone(),
                        },
                        cx,
                    )
                })
        });
        workspace.update(cx, |ws, cx| ws.show_modal(dialog, window, cx));
    }

    /// Sends `cluster::Disconnect`.
    fn disconnect(&self, cluster: &ClusterId, cx: &mut Context<Self>) {
        self.deps.dispatcher.dispatch(
            Command::ClusterDisconnect {
                cluster: cluster.clone(),
            },
            cx,
        );
    }
}
