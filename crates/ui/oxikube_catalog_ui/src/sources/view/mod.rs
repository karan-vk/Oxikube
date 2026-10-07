//! [`SourcesView`]: the GPUI entity of the kubeconfig sources screen.
//!
//! | File | Holds |
//! |---|---|
//! | `mod.rs` | the struct, loading, the change subscription, running commands |
//! | `actions.rs` | what the buttons do: pick a file or folder, paste, remove, reload |
//! | `render.rs` | the frame: header, notice, list or one of its stand-ins |
//! | `row.rs` | one row of the list |
//! | `empty.rs` | the loading, failed and empty bodies |
//! | `item.rs` | the workspace `Item` |

mod actions;
mod empty;
mod item;
mod render;
mod row;

pub use actions::removal_text;
pub use empty::{EMPTY_STEPS, EMPTY_TITLE, LOADING_TEXT};

use std::rc::Rc;

use futures::StreamExt as _;
use gpui::{
    Context, EventEmitter, FocusHandle, Focusable, Task, UniformListScrollHandle, WeakEntity,
};
use oxikube_app::command_bus::CommandOutput;
use oxikube_app::sources::SourceRow;
use oxikube_domain::OxiResult;
use oxikube_domain::command::Command;
use oxikube_workspace::{ItemEvent, Workspace};

use super::backend::SourcesBackend;
use super::model::{Notice, SourcesModel};

/// What the sources screen needs from the outside. All of it is handed in, so a test builds the
/// view over fakes and the app builds it over its service.
#[derive(Clone)]
pub struct SourcesDeps {
    /// Where rows come from and commands go.
    pub backend: Rc<dyn SourcesBackend>,
    /// The workspace whose modal layer hosts the paste dialog and the removal confirmation.
    /// `None` outside a workspace: those two actions then say so instead of opening.
    pub workspace: Option<WeakEntity<Workspace>>,
}

/// The kubeconfig sources screen. See the module docs.
pub struct SourcesView {
    pub(super) deps: SourcesDeps,
    pub(super) model: SourcesModel,
    pub(super) focus: FocusHandle,
    pub(super) scroll: UniformListScrollHandle,
    /// A command is running: the buttons wait for it. A flag, not a task slot, because the
    /// task that runs it must finish even if this view is closed and must not clear itself.
    pub(super) busy: bool,
    /// The read in flight. A newer read replaces (and so cancels) it from outside; the task
    /// never clears its own slot.
    load: Option<Task<()>>,
    /// The file picker, while it is open. Replaced from outside, never from inside.
    pub(super) picker: Option<Task<()>>,
    /// Re-reads the rows when the catalog changes. Lives as long as the view.
    _watch_changes: Task<()>,
}

impl EventEmitter<ItemEvent> for SourcesView {}

impl Focusable for SourcesView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SourcesView {
    /// Builds the view and starts reading the rows in the background. The first frame shows the
    /// loading state.
    pub fn new(deps: SourcesDeps, cx: &mut Context<Self>) -> Self {
        let mut changes = deps.backend.changes();
        let watch = cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                if this.update(cx, |this, cx| this.reload(cx)).is_err() {
                    break;
                }
            }
        });
        let mut view = Self {
            deps,
            model: SourcesModel::new(),
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            busy: false,
            load: None,
            picker: None,
            _watch_changes: watch,
        };
        view.reload(cx);
        view
    }

    /// The view model, for tests and for the status of the screen.
    pub fn model(&self) -> &SourcesModel {
        &self.model
    }

    /// Whether a command is running.
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Reads the rows again (the sources changed, or the caller asks). The previous read, if
    /// still running, is dropped.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let rows = self.deps.backend.rows(cx);
        self.load = Some(cx.spawn(async move |this, cx| {
            let result = rows.await;
            this.update(cx, |this, cx| this.finish_load(result, cx))
                .ok();
        }));
    }

    fn finish_load(&mut self, result: OxiResult<Vec<SourceRow>>, cx: &mut Context<Self>) {
        match result {
            Ok(rows) => self.model.set_rows(rows),
            Err(error) => {
                tracing::warn!(%error, "the kubeconfig sources could not be read");
                self.model.set_failed(error.message().to_owned());
            }
        }
        cx.notify();
    }

    /// Shows what a finished command said, and refreshes the rows.
    pub(super) fn show_outcome(
        &mut self,
        result: &OxiResult<CommandOutput>,
        cx: &mut Context<Self>,
    ) {
        let notice = match result {
            Ok(output) => output.message.clone().map(|text| Notice {
                text: text.into(),
                error: false,
            }),
            Err(error) => Some(Notice {
                text: error.message().to_owned().into(),
                error: true,
            }),
        };
        self.model.set_notice(notice);
        self.reload(cx);
        cx.notify();
    }

    /// Runs `commands` one after the other, then shows the last notice (or the first error) and
    /// refreshes the rows. Ignored while another run is in progress.
    pub(super) fn run_commands(&mut self, commands: Vec<Command>, cx: &mut Context<Self>) {
        if self.busy || commands.is_empty() {
            return;
        }
        self.busy = true;
        cx.notify();
        // Detached: the files and the list are edited to the end even if the screen is closed
        // in the meantime. The task clears `busy` through the weak handle and nothing else of
        // itself.
        cx.spawn(async move |this, cx| {
            let mut last: Option<OxiResult<CommandOutput>> = None;
            for command in commands {
                let Ok(task) = this.update(cx, |this, cx| this.deps.backend.run(command, cx))
                else {
                    return;
                };
                let result = task.await;
                let failed = result.is_err();
                last = Some(result);
                if failed {
                    break;
                }
            }
            this.update(cx, |this, cx| {
                this.busy = false;
                if let Some(result) = last {
                    this.show_outcome(&result, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Puts `text` in the notice line.
    pub(super) fn say(
        &mut self,
        text: impl Into<gpui::SharedString>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        self.model.set_notice(Some(Notice {
            text: text.into(),
            error,
        }));
        cx.notify();
    }
}
