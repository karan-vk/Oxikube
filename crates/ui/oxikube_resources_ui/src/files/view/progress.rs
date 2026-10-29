//! Progress: the toasts, the ticker that keeps them and the panel current, and cancel.

use gpui::Context;
use oxikube_app::files::{Direction, TransferInfo, TransferState};
use oxikube_ports::exec::transfer::PROGRESS_INTERVAL;
use oxikube_runtime::notify_coalesced;
use oxikube_workspace::{Toast, ToastAction};

use super::FilesView;
use crate::files::format::size_text;

impl FilesView {
    /// Shows `toast` in the cluster tab this view is in.
    pub(crate) fn toast(&self, toast: Toast, cx: &mut Context<Self>) {
        if let Some(workspace) = self.workspace.as_ref().and_then(|ws| ws.upgrade()) {
            workspace.update(cx, |workspace, cx| {
                workspace.show_toast(toast, cx);
            });
        }
    }

    fn dismiss_toast(&self, key: &str, cx: &mut Context<Self>) {
        if let Some(workspace) = self.workspace.as_ref().and_then(|ws| ws.upgrade()) {
            workspace.update(cx, |workspace, cx| {
                workspace.dismiss_toast_key(key, cx);
            });
        }
    }

    /// Whether `info` is a transfer of this view's pod.
    pub(crate) fn is_mine(&self, info: &TransferInfo) -> bool {
        let ns = self.pod.namespace().unwrap_or("default");
        let prefix = format!("{ns}/{}", self.pod.name);
        info.place == prefix || info.place.starts_with(&format!("{prefix} ("))
    }

    /// Starts the progress ticker if it is not running: at most every [`PROGRESS_INTERVAL`] it
    /// redraws the panel and updates the toasts, and it stops when nothing runs.
    pub(crate) fn watch_transfers(&mut self, cx: &mut Context<Self>) {
        if self.ticking {
            return;
        }
        self.ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PROGRESS_INTERVAL).await;
                let keep = this
                    .update(cx, |view, cx| view.tick_transfers(cx))
                    .unwrap_or(false);
                if !keep {
                    this.update(cx, |view, _| view.ticking = false).ok();
                    break;
                }
            }
        })
        .detach();
    }

    /// One tick: when the board changed, redraws (coalesced) and brings the progress toasts up to
    /// date. `true` while this pod has a transfer running.
    pub(crate) fn tick_transfers(&mut self, cx: &mut Context<Self>) -> bool {
        let board = self.deps.runner.service().board().clone();
        let version = board.version();
        let transfers: Vec<TransferInfo> = board
            .snapshot()
            .into_iter()
            .filter(|info| self.is_mine(info))
            .collect();
        if version != self.seen_version {
            self.seen_version = version;
            self.update_progress_toasts(&transfers, &board, cx);
            notify_coalesced(cx);
        }
        keep_ticking(self.jobs, &transfers)
    }

    fn update_progress_toasts(
        &mut self,
        transfers: &[TransferInfo],
        board: &oxikube_app::files::TransferBoard,
        cx: &mut Context<Self>,
    ) {
        for info in transfers {
            let key = format!("files/progress/{}", transfer_number(info));
            if info.state == TransferState::Running {
                let id = info.id;
                let board = board.clone();
                let toast = Toast::info(progress_text(info))
                    .key(key)
                    .persistent()
                    .action(ToastAction::new("Cancel", move |_, _| {
                        board.cancel(id);
                    }));
                self.toast(toast, cx);
                self.toasted.insert(info.id);
            } else if self.toasted.remove(&info.id) {
                // The result toast replaces it.
                self.dismiss_toast(&key, cx);
            }
        }
    }

    /// Stops a running transfer (the panel's button).
    pub fn cancel_transfer(&mut self, id: oxikube_app::files::TransferId, cx: &mut Context<Self>) {
        self.deps.runner.service().board().cancel(id);
        cx.notify();
    }

    /// Forgets the finished transfers listed in the panel.
    pub fn clear_transfers(&mut self, cx: &mut Context<Self>) {
        self.deps.runner.service().board().clear_finished();
        self.toasted.clear();
        cx.notify();
    }
}

/// Whether the ticker goes on: a batch is still in flight, or a transfer of the pod runs.
pub(crate) fn keep_ticking(jobs: usize, transfers: &[TransferInfo]) -> bool {
    jobs > 0 || transfers.iter().any(|info| !info.state.is_finished())
}

/// A number naming the transfer in toast keys.
fn transfer_number(info: &TransferInfo) -> String {
    info.id.number().to_string()
}

/// `Downloading motd: 4.2 MiB of 11.0 MiB (38%) at 2.1 MiB/s`.
pub(crate) fn progress_text(info: &TransferInfo) -> String {
    let verb = match info.direction {
        Direction::Download => "Downloading",
        Direction::Upload => "Uploading",
    };
    let mut text = format!("{verb} {}: {}", info.local, size_text(info.bytes));
    if let Some(total) = info.total {
        text.push_str(&format!(" of {}", size_text(total)));
        if let Some(fraction) = info.fraction() {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a percentage shown to whole numbers, from a clamped fraction"
            )]
            let percent = (fraction * 100.0) as u32;
            text.push_str(&format!(" ({percent}%)"));
        }
    }
    if info.speed > 1.0 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a speed shown to a few digits"
        )]
        let speed = info.speed as u64;
        text.push_str(&format!(" at {}/s", size_text(speed)));
    }
    text
}
