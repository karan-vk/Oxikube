//! Send to agent (`a`, `logs::SendToAgent`): the selected lines, else the lines on screen, queued
//! as context for the hosted agent with where they came from.
//!
//! The lines are read from the view's own buffer (nothing is asked of the cluster), written with
//! their server time (and pod, in a multi-pod view), masked of secrets and bounded
//! ([`selection_context`]), and pushed into the app's [`PendingContext`](oxikube_app::context::PendingContext).
//! The agent panel (E27) drains the queue once it exists; until then the toast says the lines wait.

use gpui::Context;
use oxikube_app::context::{ContextSource, Sent, selection_context};
use oxikube_app::logs::export::{ExportFormat, copy_text};
use oxikube_workspace::Toast;

use super::LogView;
use super::text::lines_of;

/// The most a send reads from the buffer; the block is cut to 64 KiB after masking.
pub const SEND_LIMIT_BYTES: usize = 128 * 1024;

impl LogView {
    /// Queues the selected lines (else the lines on screen) for the agent. A toast says how many
    /// lines went and whether the agent panel has them yet.
    pub fn send_to_agent(&mut self, cx: &mut Context<Self>) {
        let (Some(session), Some(seqs)) = (self.session.as_ref(), self.copy_seqs()) else {
            self.toast(Toast::info("There are no lines to send."), cx);
            return;
        };
        let format = ExportFormat {
            timestamps: true,
            pod_prefix: self.aggregate.is_some(),
        };
        let spec = self.spec_for(seqs, format);
        let copied = copy_text(&session.reader(), &spec, SEND_LIMIT_BYTES);
        if copied.lines == 0 {
            self.toast(Toast::info("There are no lines to send."), cx);
            return;
        }
        let cluster_name = self
            .deps
            .sessions
            .get(&self.target.cluster)
            .map_or_else(|| self.target.cluster.to_string(), |s| s.title().to_owned());
        let source = ContextSource {
            cluster: self.target.cluster.clone(),
            cluster_name,
            namespace: self
                .target
                .namespace
                .as_deref()
                .unwrap_or_default()
                .to_owned(),
            subject: self.subject(),
            container: self.options.container.clone(),
            span: copied.span,
            lines: usize::try_from(copied.lines).unwrap_or(usize::MAX),
        };
        let item = selection_context(source, &copied.text);
        let lines = item.source.lines;
        let toast = match self.deps.agent.send(item) {
            Sent::Delivered => {
                Toast::success(format!("Sent {} to the agent", lines_of(lines as u64)))
            }
            Sent::Queued(waiting) => Toast::info(format!(
                "Queued {} for the agent ({waiting} waiting). They are delivered when the agent \
                 panel opens.",
                lines_of(lines as u64)
            )),
        };
        self.toast(toast.key("logs-send-to-agent"), cx);
    }
}
