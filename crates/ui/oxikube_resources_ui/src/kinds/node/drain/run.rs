//! The drain dialog's run: start `node::Drain`, follow its steps, cancel it, uncordon after.

use futures::StreamExt as _;
use gpui::{Context, DismissEvent};
use oxikube_app::nodes::DrainStart;
use oxikube_domain::command::Command;
use oxikube_domain::drain::{DrainBoard, DrainPhase};
use oxikube_runtime::{notify_coalesced, spawn_kube};
use oxikube_workspace::Toast;

use super::dialog::{DrainDialog, DrainStage};
use super::rows::progress_lines;

/// Most steps applied in one update; a burst is split so a frame never waits on a long batch.
const STEP_BATCH: usize = 256;

impl DrainDialog {
    /// Starts the drain: follows its steps into the board and runs `node::Drain` on the Tokio
    /// bridge with the typed name. Does nothing until [`can_start`](Self::can_start).
    pub fn start(&mut self, cx: &mut Context<Self>) {
        if !self.can_start(cx) {
            return;
        }
        let Ok((grace, timeout)) = self.numbers(cx) else {
            return;
        };
        self.flags.grace_period_secs = grace;
        self.flags.timeout_secs = timeout;
        let typed = self.typed.read(cx).value().to_string();
        let DrainStart { progress, run } = self.flow.start(&self.target, self.flags.clone(), typed);
        self.stage = DrainStage::Draining;
        self.error = None;
        self.board = DrainBoard::new();
        self.lines = progress_lines(&self.board);
        self.follow = Some(cx.spawn(async move |this, cx| {
            let mut batches = progress.ready_chunks(STEP_BATCH);
            while let Some(steps) = batches.next().await {
                let delivered = this.update(cx, |this, cx| {
                    for step in &steps {
                        this.board.apply(step);
                    }
                    this.lines = progress_lines(&this.board);
                    notify_coalesced(cx);
                });
                if delivered.is_err() {
                    break;
                }
            }
        }));
        let task = spawn_kube(cx, run);
        self.run = Some(cx.spawn(async move |this, cx| {
            let result = match task.await {
                Ok(result) => result.map_err(|error| error.message().to_owned()),
                Err(error) => Err(error.to_string()),
            };
            this.update(cx, |this, cx| this.finished(result, cx)).ok();
        }));
        cx.notify();
    }

    fn finished(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
        if self.stage != DrainStage::Draining {
            return;
        }
        self.stage = DrainStage::Done;
        match result {
            Ok(message) => {
                self.message = Some(message.clone().into());
                self.workspace
                    .update(cx, |workspace, cx| {
                        workspace.show_toast(Toast::success(message), cx)
                    })
                    .ok();
            }
            Err(error) => {
                self.board.fail(error.clone());
                self.error = Some(error.into());
            }
        }
        self.lines = progress_lines(&self.board);
        cx.notify();
    }

    /// Stops the running drain: the dispatch is dropped, so no further pod is evicted (an
    /// eviction already sent stays sent) and the guard audits it as cancelled. The node stays
    /// cordoned; the dialog then offers to uncordon it.
    pub fn cancel_drain(&mut self, cx: &mut Context<Self>) {
        if self.stage != DrainStage::Draining {
            return;
        }
        self.run = None;
        self.stage = DrainStage::Done;
        self.board.cancel();
        self.lines = progress_lines(&self.board);
        self.message = Some("Drain cancelled. The node stays cordoned.".into());
        cx.notify();
    }

    /// Whether the dialog offers "Uncordon": the drain ended without every pod gone and the node
    /// was schedulable before it.
    pub fn offers_uncordon(&self) -> bool {
        self.stage == DrainStage::Done
            && self.board.was_cordoned() == Some(false)
            && !matches!(
                self.board.phase(),
                DrainPhase::Finished(summary) if summary.is_complete()
            )
    }

    /// Sends `node::Uncordon` (the guard asks its one click) and closes the dialog.
    pub fn uncordon(&mut self, cx: &mut Context<Self>) {
        if !self.offers_uncordon() {
            return;
        }
        let command = Command::NodeUncordon {
            target: self.target.clone(),
        };
        cx.emit(DismissEvent);
        self.dispatcher.dispatch(command, cx);
    }
}
