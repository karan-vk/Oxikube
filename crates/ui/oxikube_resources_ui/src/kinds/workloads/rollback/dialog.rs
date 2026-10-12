//! [`RollbackDialog`]: the history, the chosen revision, and the run.

use std::sync::Arc;

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, SharedString, Task,
    WeakEntity, Window,
};
use oxikube_app::{RollbackReport, RollbackRunner, RolloutHistory, RolloutService};
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ResourceRef;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::modal::{ModalPlacement, ModalView};
use oxikube_workspace::{Toast, Workspace};

use super::rows::HistoryRow;

/// The most revisions the table lists, newest first; a Deployment may keep a thousand
/// (`revisionHistoryLimit`) and the dialog is a picker, not an archive.
pub const MAX_ROWS: usize = 100;

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackStage {
    /// Reading the history.
    Loading,
    /// Showing it. Nothing has been sent.
    Choosing,
    /// `workload::Rollback` is running.
    RollingBack,
}

/// The rollback dialog. See the [module docs](super).
pub struct RollbackDialog {
    pub(super) target: ResourceRef,
    runner: RollbackRunner,
    pub(super) history: Option<RolloutHistory>,
    pub(super) rows: Vec<HistoryRow>,
    /// Older revisions the table does not list.
    pub(super) hidden: usize,
    /// The confirmation line, made once.
    pub(super) tier: SharedString,
    pub(super) selected: Option<i64>,
    pub(super) stage: RollbackStage,
    /// Why the last read or run failed: the server's message.
    pub(super) error: Option<SharedString>,
    workspace: WeakEntity<Workspace>,
    pub(super) focus: FocusHandle,
    /// The read of the history; replaced, never cleared from inside.
    _history_task: Task<()>,
}

impl EventEmitter<DismissEvent> for RollbackDialog {}

impl Focusable for RollbackDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for RollbackDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Escape and a click outside cancel while choosing; while the rollback runs the dialog stays
    /// so its outcome is not lost.
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage != RollbackStage::RollingBack
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage != RollbackStage::RollingBack
    }
}

impl RollbackDialog {
    /// A dialog for `target`. It reads the history through `service` on the Tokio bridge and
    /// shows the table once that answers; submitting runs `runner`.
    pub fn new(
        target: ResourceRef,
        service: Arc<RolloutService>,
        runner: RollbackRunner,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let read = {
            let target = target.clone();
            spawn_kube(cx, async move { service.history(&target).await })
        };
        let history_task = cx.spawn_in(window, async move |this, cx| {
            let history = read.await.unwrap_or_else(|error| Err(error.into()));
            this.update(cx, |this, cx| this.loaded(history, cx)).ok();
        });
        Self {
            target,
            runner,
            history: None,
            rows: Vec::new(),
            hidden: 0,
            tier: Self::tier_line().into(),
            selected: None,
            stage: RollbackStage::Loading,
            error: None,
            workspace,
            focus: cx.focus_handle(),
            _history_task: history_task,
        }
    }

    /// The history arrived (or could not be read).
    fn loaded(&mut self, history: OxiResult<RolloutHistory>, cx: &mut Context<Self>) {
        match history {
            Ok(history) => {
                let now = jiff::Timestamp::now();
                // The newest `MAX_ROWS`, and further back if the revision to start on is older
                // (a ReplicaSet row stands for its own revision).
                let shown = history
                    .preselected
                    .and_then(|wanted| history.revisions.iter().position(|r| r.revision == wanted))
                    .map_or(MAX_ROWS, |at| MAX_ROWS.max(at + 1));
                self.rows = history
                    .revisions
                    .iter()
                    .take(shown)
                    .map(|revision| HistoryRow::of(revision, now))
                    .collect();
                self.hidden = history.revisions.len().saturating_sub(self.rows.len());
                self.selected = history
                    .preselected
                    .filter(|wanted| self.rows.iter().any(|row| row.revision == *wanted));
                self.history = Some(history);
            }
            Err(error) => self.error = Some(error.message().to_owned().into()),
        }
        self.stage = RollbackStage::Choosing;
        cx.notify();
    }

    /// Focuses the dialog, so its keys (up, down, enter, escape) work at once.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    /// Where the dialog is.
    pub fn stage(&self) -> RollbackStage {
        self.stage
    }

    /// The object the dialog was opened on.
    pub fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// The history, once read.
    pub fn history(&self) -> Option<&RolloutHistory> {
        self.history.as_ref()
    }

    /// The table's rows, newest revision first.
    pub fn rows(&self) -> &[HistoryRow] {
        &self.rows
    }

    /// How many older revisions are not in the table.
    pub fn hidden(&self) -> usize {
        self.hidden
    }

    /// The revision chosen to go back to.
    pub fn selected(&self) -> Option<i64> {
        self.selected
    }

    /// The row of the chosen revision.
    pub fn selected_row(&self) -> Option<&HistoryRow> {
        self.rows
            .iter()
            .find(|row| Some(row.revision) == self.selected)
    }

    /// The message under the table, if a read or a run failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether the workload is a paused Deployment, which kubectl refuses to roll back.
    pub fn paused(&self) -> bool {
        self.history.as_ref().is_some_and(|history| history.paused)
    }

    /// Chooses `revision`. Ignored while the rollback runs or for a revision not in the table.
    pub fn select(&mut self, revision: i64, cx: &mut Context<Self>) {
        if self.stage == RollbackStage::RollingBack
            || !self.rows.iter().any(|row| row.revision == revision)
        {
            return;
        }
        self.selected = Some(revision);
        self.error = None;
        cx.notify();
    }

    /// Moves the choice `delta` rows down the table (up for a negative one), staying inside it.
    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let at = self
            .rows
            .iter()
            .position(|row| Some(row.revision) == self.selected);
        let next = match at {
            Some(at) => at.saturating_add_signed(delta).min(self.rows.len() - 1),
            None => 0,
        };
        let revision = self.rows[next].revision;
        self.select(revision, cx);
    }

    /// Whether the button can run now: a revision other than the running one is chosen on a
    /// workload that is not paused.
    pub fn can_submit(&self) -> bool {
        self.stage == RollbackStage::Choosing
            && !self.paused()
            && self.selected_row().is_some_and(|row| !row.current)
    }

    /// Cancels while not running: nothing is sent.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage != RollbackStage::RollingBack {
            cx.emit(DismissEvent);
        }
    }

    /// Rolls back: runs `workload::Rollback` for the chosen revision on the Tokio bridge. Does
    /// nothing when [`can_submit`](Self::can_submit) is false.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        let (Some(revision), true) = (self.selected, self.can_submit()) else {
            return;
        };
        // The workload the history belongs to: for a ReplicaSet, its Deployment.
        let workload = self
            .history
            .as_ref()
            .map_or_else(|| self.target.clone(), |history| history.workload.clone());
        self.stage = RollbackStage::RollingBack;
        self.error = None;
        cx.notify();
        let runner = self.runner.clone();
        let workspace = self.workspace.clone();
        // Detached: the command must finish even if the dialog goes away meanwhile (it is already
        // audited as it runs); the task ends itself and clears nothing.
        cx.spawn(async move |this, cx| {
            let run = spawn_kube(
                cx,
                async move { runner.run(&workload, Some(revision)).await },
            );
            let result = match run.await {
                Ok(result) => result.map_err(|error| error.message().to_owned()),
                Err(error) => Err(error.to_string()),
            };
            let delivered = this.update(cx, |this, cx| this.finished(&result, cx));
            if delivered.is_err() {
                // The dialog is gone: say how it went where the user still looks.
                let toast = match &result {
                    Ok(report) => Toast::success(report.message.clone()),
                    Err(message) => Toast::error(message.clone()),
                };
                workspace
                    .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn finished(&mut self, result: &Result<RollbackReport, String>, cx: &mut Context<Self>) {
        self.stage = RollbackStage::Choosing;
        match result {
            Ok(report) => {
                self.workspace
                    .update(cx, |workspace, cx| {
                        workspace.show_toast(Toast::success(report.message.clone()), cx);
                    })
                    .ok();
                cx.emit(DismissEvent);
            }
            Err(message) => {
                self.error = Some(message.clone().into());
                cx.notify();
            }
        }
    }
}
