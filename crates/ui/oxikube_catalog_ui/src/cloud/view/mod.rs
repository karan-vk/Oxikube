//! [`CloudView`]: the GPUI entity of the catalog's Cloud section.
//!
//! | File | Holds |
//! |---|---|
//! | `mod.rs` | the struct, the event subscription, the first refresh, running commands |
//! | `actions.rs` | what the buttons do: refresh, add, remove, cancel, copy a command |
//! | `render.rs` | the frame: header, notice, the cards and the selected provider |
//! | `cards.rs` | one card per detected provider |
//! | `rows.rs` | the filter bar, the setup banner and one cluster row |
//! | `item.rs` | the workspace `Item` |

mod actions;
mod cards;
mod item;
mod render;
mod rows;

pub use render::{EMPTY_STEPS, EMPTY_TITLE, LOADING_TEXT};
pub use rows::IN_CATALOG;

use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, SharedString, Subscription, Task,
    UniformListScrollHandle,
};
use oxikube_app::cloud::CloudDiscoveryEvent;
use oxikube_domain::command::{CloudRefreshMode, Command};
use oxikube_keymap::{KeyContextBuilder, KeyContextual, contexts};
use oxikube_ports::ClockPort;
use oxikube_runtime::NotifyCoalescedExt as _;
use oxikube_workspace::ItemEvent;

use super::backend::CloudBackend;
use super::model::CloudModel;

/// The key-context flag set while a discovery runs.
const LOADING_FLAG: &str = "Loading";

/// What the Cloud section needs from the outside. All of it is handed in, so a test builds the
/// view over a scripted backend and the app builds it over its service.
#[derive(Clone)]
pub struct CloudDeps {
    /// Where commands go and discovery comes from.
    pub backend: Rc<dyn CloudBackend>,
    /// The clock for "refreshed 3m ago".
    pub clock: Arc<dyn ClockPort>,
}

/// A one-line message under the header: what the last action did, or why it failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// The text.
    pub text: SharedString,
    /// The command that fixes it, when there is one (the notice offers to copy it).
    pub command: Option<String>,
    /// Shown as an error.
    pub error: bool,
}

/// The catalog's Cloud section. See the module docs of [`crate::cloud`].
pub struct CloudView {
    pub(super) deps: CloudDeps,
    pub(super) model: CloudModel,
    pub(super) focus: FocusHandle,
    pub(super) scroll: UniformListScrollHandle,
    pub(super) notice: Option<Notice>,
    /// The key of the cluster whose removal waits for a second click.
    pub(super) confirm_remove: Option<String>,
    /// A cluster add or remove is running: the buttons wait for it. A flag, not a task slot,
    /// because the command must finish even if this view is closed and must not clear itself.
    pub(super) editing: bool,
    /// The first frame was drawn and the first refresh asked for.
    pub(super) started: bool,
    /// Follows the discovery events. Lives as long as the view.
    _watch: Task<()>,
    /// Stops a running discovery when the view is closed.
    _on_release: Subscription,
}

impl EventEmitter<ItemEvent> for CloudView {}

impl Focusable for CloudView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl KeyContextual for CloudView {
    const KEY_CONTEXT: &'static str = contexts::CATALOG_CLOUD;

    fn extend_key_context(&self, context: &mut KeyContextBuilder) {
        context.flag_if(self.model.any_running(), LOADING_FLAG);
    }
}

impl CloudView {
    /// Builds the view. The first frame shows the section empty; the first refresh (the cache
    /// answers at once, the CLIs run only when it is stale) starts after that frame, never
    /// before it (see `render`).
    pub fn new(deps: CloudDeps, cx: &mut Context<Self>) -> Self {
        let mut events = deps.backend.watch();
        let watch = cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this.update(cx, |this, cx| this.apply(&event, cx)).is_err() {
                    break;
                }
            }
        });
        let on_release = cx.on_release(|this, cx| {
            if this.model.any_running() {
                this.deps
                    .backend
                    .run(Command::CloudCancel { provider: None }, cx)
                    .detach();
            }
        });
        Self {
            deps,
            model: CloudModel::new(),
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            notice: None,
            confirm_remove: None,
            editing: false,
            started: false,
            _watch: watch,
            _on_release: on_release,
        }
    }

    /// The view model, for tests and for the status of the screen.
    pub fn model(&self) -> &CloudModel {
        &self.model
    }

    /// The notice under the header, if any.
    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    /// Whether a cluster add or remove is running.
    pub fn is_editing(&self) -> bool {
        self.editing
    }

    fn apply(&mut self, event: &CloudDiscoveryEvent, cx: &mut Context<Self>) {
        let now = self.deps.clock.now();
        self.model.apply(event, now);
        // A burst of scope results redraws once per frame.
        cx.notify_coalesced();
    }

    /// Asks for the first refresh. Called from the first `render`, so it runs after the first
    /// frame is on screen: the catalog never waits for a CLI.
    pub(super) fn start(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        // Spawned, so it runs after the frame that asked for it.
        cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| {
                this.reload_added(cx);
                this.refresh_all(CloudRefreshMode::IfStale, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Puts `notice` under the header.
    pub(super) fn say(&mut self, notice: Notice, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        cx.notify();
    }
}
