//! [`JumpHost`]: the jump bar of one window, and how `palette::OpenJump` and the history commands
//! reach it.
//!
//! The binary builds one host per window when it mounts the cluster UI ([`JumpHost::install`]).
//! Every door ends in [`JumpHost::apply`]: the GPUI actions (`:` in a table, `[`, `]`, `-`), and
//! the bus commands of the same names (the palette, menus, agents) through [`JumpSink`]. The bar
//! itself is [`JumpBar`](super::JumpBar); this is what it runs a confirmed line on.
//!
//! A confirmed line is a [`JumpPlan`]: commands for the window's [`CommandDispatcher`], the path
//! keys and buttons take, so a jump is guarded, audited and visible to an agent like any click. They
//! are sent after the bar has closed and given the focus back, so the view a jump opens keeps it.
//! A jump to a context that is not connected sends the connect and the rest once the session is
//! up ([`super::connect`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{AnyWindowHandle, App, Global, Subscription, Task, WeakEntity, Window, WindowId};
use oxikube_app::ClusterSessionManager;
use oxikube_app::search::jump::{self, HistoryStep, JumpHistory, JumpPlan};
use oxikube_workspace::modal::ModalLayerEvent;
use oxikube_workspace::{CommandDispatcher, Toast, Workspace};

use super::bar::JumpBar;
use super::delegate::JumpDelegate;
use super::request::JumpRequest;
use super::sources::{JumpSources, LiveEnv, Loaded};
use super::{Back, Forward, Last, OpenJump, connect};

/// State the bar, the host and the connect wait share.
#[derive(Default)]
pub(super) struct Shared {
    /// The lines run this session.
    pub history: RefCell<JumpHistory>,
    /// Contexts and namespaces found by earlier opens.
    pub loaded: RefCell<Loaded>,
    /// The plan a confirm left for the host to run once the bar has closed.
    pub outbox: RefCell<Option<JumpPlan>>,
    /// The wait for a connecting cluster; a newer jump replaces it.
    waiting: RefCell<Option<Task<()>>>,
}

impl Shared {
    /// A confirmed line: it goes into the history and its commands wait for the bar to close.
    pub fn submit(&self, plan: JumpPlan) {
        if let Some(line) = &plan.record {
            self.history.borrow_mut().record(line);
        }
        *self.outbox.borrow_mut() = Some(plan);
    }
}

/// What runs a [`JumpPlan`]: the window's dispatcher, the sessions to wait on, the workspace that
/// shows the toast of a jump that could not be made.
#[derive(Clone)]
pub(super) struct Runner {
    dispatcher: Rc<dyn CommandDispatcher>,
    sessions: ClusterSessionManager,
    workspace: WeakEntity<Workspace>,
    shared: Rc<Shared>,
}

impl Runner {
    /// Sends a plan's commands in order, and the rest of it once its cluster is connected.
    pub fn run(&self, plan: &JumpPlan, window: &mut Window, cx: &mut App) {
        for command in &plan.commands {
            self.dispatcher.dispatch(command.clone(), cx);
        }
        // A newer jump replaces (and so cancels) the wait of an older one: the user went on.
        let waiting = plan.after_connect.as_ref().map(|after| {
            connect::when_connected(
                after.clone(),
                self.dispatcher.clone(),
                self.sessions.clone(),
                self.workspace.clone(),
                window,
                cx,
            )
        });
        *self.shared.waiting.borrow_mut() = waiting;
    }

    fn toast(&self, toast: Toast, cx: &mut App) {
        show_toast(&self.workspace, toast, cx);
    }
}

/// Shows `toast` in `workspace` (nothing when the window is gone).
pub(super) fn show_toast(workspace: &WeakEntity<Workspace>, toast: Toast, cx: &mut App) {
    workspace
        .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
        .ok();
}

/// The jump bar of one window. See the [module docs](super).
pub struct JumpHost {
    workspace: WeakEntity<Workspace>,
    runner: Runner,
    sources: JumpSources,
    shared: Rc<Shared>,
    /// Sends the confirmed plan when the bar closes; replaced by the next open.
    sender: RefCell<Option<Subscription>>,
}

impl JumpHost {
    /// The jump bar of the window whose main workspace is `workspace`.
    pub fn new(
        workspace: &gpui::Entity<Workspace>,
        dispatcher: Rc<dyn CommandDispatcher>,
        sources: JumpSources,
    ) -> Self {
        let shared = Rc::<Shared>::default();
        Self {
            workspace: workspace.downgrade(),
            runner: Runner {
                dispatcher,
                sessions: sources.sessions.clone(),
                workspace: workspace.downgrade(),
                shared: shared.clone(),
            },
            sources,
            shared,
            sender: RefCell::new(None),
        }
    }

    /// Makes this the jump bar of `window`: `:` and the history keys act on it.
    pub fn install(self: &Rc<Self>, window: &Window, cx: &mut App) {
        let id = window.window_handle().window_id();
        let open: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
        let hosts = cx.default_global::<Hosts>();
        hosts.0.retain(|window, _| open.contains(window));
        hosts.0.insert(id, self.clone());
    }

    /// The lines run so far (for the tests and the help overlay).
    pub fn history(&self) -> JumpHistory {
        self.shared.history.borrow().clone()
    }

    /// Does what a door asked, in `window`.
    pub fn apply(&self, request: JumpRequest, window: &mut Window, cx: &mut App) {
        match request {
            JumpRequest::Open => self.open(window, cx),
            JumpRequest::Step(step) => self.step(step, window, cx),
        }
    }

    /// Opens the bar over the focused view, or closes it when it is open.
    ///
    /// The first frame is the bar with the aliases of the shown cluster; the contexts and the
    /// namespaces are read in the background and the bar picks them up when they arrive.
    pub fn open(&self, window: &mut Window, cx: &mut App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        if open_bar(&workspace, cx).is_some() {
            workspace.update(cx, |workspace, cx| workspace.hide_modal(window, cx));
            return;
        }
        let env = LiveEnv::snapshot(&self.sources, &self.shared.loaded.borrow(), cx);
        let delegate = JumpDelegate::new(Rc::new(env), self.shared.clone());
        let sources = self.sources.clone();
        let shared = self.shared.clone();
        self.send_when_closed(&workspace, window.window_handle(), cx);
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_modal(window, cx, move |window, cx| {
                JumpBar::new(delegate, sources, shared, window, cx)
            });
        });
    }

    /// `[`, `]`, `-`: runs an earlier line again. A line that cannot be planned now (its cluster
    /// is gone) says so in a toast.
    pub fn step(&self, step: HistoryStep, window: &mut Window, cx: &mut App) {
        let line = {
            let mut history = self.shared.history.borrow_mut();
            match step {
                HistoryStep::Back => history.back(),
                HistoryStep::Forward => history.forward(),
                HistoryStep::Last => history.last(),
            }
            .map(str::to_owned)
        };
        let Some(line) = line else {
            let message = match step {
                HistoryStep::Back => "Nothing earlier in the jump history.",
                HistoryStep::Forward => "Nothing later in the jump history.",
                HistoryStep::Last => "There is no previous view to go back to.",
            };
            self.runner.toast(Toast::info(message), cx);
            return;
        };
        let env = LiveEnv::snapshot(&self.sources, &self.shared.loaded.borrow(), cx);
        match jump::plan(&line, &env) {
            Ok(plan) => self.runner.run(&plan, window, cx),
            Err(error) => self
                .runner
                .toast(Toast::warning(format!("`{line}`: {error}")), cx),
        }
    }

    /// Runs the plan a confirm leaves once the modal layer reports the bar closed.
    ///
    /// The layer hands the focus back in a step it queues just before it announces the close, so
    /// by the time this runs the view the bar opened over has the focus again and what the plan
    /// opens is not taken from it. Escape leaves the outbox empty and runs nothing.
    fn send_when_closed(
        &self,
        workspace: &gpui::Entity<Workspace>,
        window: AnyWindowHandle,
        cx: &mut App,
    ) {
        let layer = workspace.read(cx).modal_layer().clone();
        let runner = self.runner.clone();
        let shared = self.shared.clone();
        let subscription = cx.subscribe(&layer, move |_, event: &ModalLayerEvent, cx| {
            if *event != ModalLayerEvent::Hidden {
                return;
            }
            let Some(plan) = shared.outbox.borrow_mut().take() else {
                return;
            };
            let runner = runner.clone();
            cx.defer(move |cx| {
                window
                    .update(cx, |_, window, cx| runner.run(&plan, window, cx))
                    .ok();
            });
        });
        *self.sender.borrow_mut() = Some(subscription);
    }

    /// Applies each request that arrives on `requests` (the sink [`super::register_commands`] was given)
    /// in `window` until the window closes. Hold the returned task as long as the window lives.
    pub fn serve(
        self: &Rc<Self>,
        mut requests: UnboundedReceiver<JumpRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        let host = self.clone();
        window.spawn(cx, async move |cx| {
            while let Some(request) = requests.next().await {
                let applied = cx.update(|window, cx| host.apply(request, window, cx));
                if applied.is_err() {
                    break;
                }
            }
        })
    }
}

/// The bar open in `workspace`'s modal layer, if any.
pub(super) fn open_bar(
    workspace: &gpui::Entity<Workspace>,
    cx: &App,
) -> Option<gpui::Entity<JumpBar>> {
    workspace
        .read(cx)
        .modal_layer()
        .read(cx)
        .active_modal::<JumpBar>()
}

/// The jump bars of the open windows.
#[derive(Default)]
struct Hosts(HashMap<WindowId, Rc<JumpHost>>);

impl Global for Hosts {}

/// The host installed for the active window; with no active window (the OS has not told us yet,
/// a test), the only window that has one.
fn active_host(cx: &App) -> Option<(AnyWindowHandle, Rc<JumpHost>)> {
    let hosts = cx.try_global::<Hosts>()?;
    let pick = |window: AnyWindowHandle| {
        hosts
            .0
            .get(&window.window_id())
            .map(|host| (window, host.clone()))
    };
    if let Some(found) = cx.active_window().and_then(pick) {
        return Some(found);
    }
    let mut windows = cx.windows().into_iter().filter_map(pick);
    let only = windows.next()?;
    windows.next().is_none().then_some(only)
}

/// Binds the jump actions to the host of the active window. The keys are in the keymap files.
/// Idempotent.
pub(super) fn register_actions(cx: &mut App) {
    if cx.has_global::<ActionsRegistered>() {
        return;
    }
    cx.set_global(ActionsRegistered);
    cx.on_action(|_: &OpenJump, cx| run_on_active(JumpRequest::Open, cx));
    cx.on_action(|_: &Back, cx| run_on_active(JumpRequest::Step(HistoryStep::Back), cx));
    cx.on_action(|_: &Forward, cx| run_on_active(JumpRequest::Step(HistoryStep::Forward), cx));
    cx.on_action(|_: &Last, cx| run_on_active(JumpRequest::Step(HistoryStep::Last), cx));
}

fn run_on_active(request: JumpRequest, cx: &mut App) {
    // Action handlers run while the window that dispatched them is borrowed: act once that
    // update has ended, with the focus still where the key found it.
    cx.defer(move |cx| {
        let Some((window, host)) = active_host(cx) else {
            return;
        };
        window
            .update(cx, |_, window, cx| host.apply(request, window, cx))
            .ok();
    });
}

struct ActionsRegistered;

impl Global for ActionsRegistered {}
