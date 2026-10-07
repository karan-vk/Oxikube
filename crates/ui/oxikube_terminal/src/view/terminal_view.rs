//! [`TerminalView`]: one terminal as a GPUI entity, owning its [`TerminalState`] (grid, backend,
//! tasks) for as long as its tab lives. Moving the tab between panes and the dock moves this
//! entity: the backend and the grid are never recreated.

use std::path::PathBuf;

use gpui::{AppContext as _, Context, Entity, FocusHandle, SharedString, Subscription, Task};
use oxikube_domain::OxiResult;
use oxikube_ports::{ExitStatus, TerminalBackend};
use oxikube_workspace::{ClusterMark, ItemEvent};

use super::bell::Bell;
use super::descriptor::{BackendDescriptor, tab_title};
use super::find::Find;
use super::lifecycle::{Failure, Lifecycle, Signal};
use super::services::TerminalServices;
use crate::backend::local::DEFAULT_SIZE;
use crate::element::TerminalElementState;
use crate::settings::TerminalSettings;
use crate::state::{TerminalEvent, TerminalState};

/// Where a terminal's process is.
pub(super) enum Phase {
    /// The launcher is starting it.
    Starting,
    /// It runs (or ran: the state keeps the exit status and the screen).
    Running(Entity<TerminalState>),
    /// It could not start; the message says why (never output or credentials).
    Failed(SharedString),
    /// The tab was closed: the process was ended and the state released.
    Closed,
}

/// A terminal tab: a [`BackendDescriptor`], the process started from it, and the element state.
/// See the module docs.
pub struct TerminalView {
    pub(super) descriptor: BackendDescriptor,
    pub(super) services: TerminalServices,
    pub(super) phase: Phase,
    /// What the user sees of the process's life (running, disconnected, exited): drives the
    /// banner and whether input is dimmed. `phase` owns the resources, this the meaning.
    pub(super) lifecycle: Lifecycle,
    pub(super) element: TerminalElementState,
    pub(super) focus: FocusHandle,
    /// The title before the process sets one (the program or the pod).
    default_title: SharedString,
    /// The title the process set, cleaned and cut.
    pub(super) process_title: Option<SharedString>,
    pub(super) mark: Option<ClusterMark>,
    /// Find in scrollback (E09-S11).
    pub(super) find: Find,
    /// The bell's flash and the cursor's blink clock (E09-S11).
    pub(super) bell: Bell,
    /// The directory the shell was in when the tab closed. The tab's own close button ends the
    /// session before the workspace saves the reopen-closed entry, so the entry reads it here.
    closed_dir: Option<PathBuf>,
    /// Keeps `mark` current as the cluster's read-only flag and colour change.
    _follow_mark: Option<Task<()>>,
    /// Starts the process. Never cleared from inside itself; dropped (cancelling a launch in
    /// flight) when the tab closes.
    pub(super) launch: Option<Task<()>>,
    pub(super) subscriptions: Vec<Subscription>,
}

impl TerminalView {
    /// A terminal running what `descriptor` describes, started at once through `services`'
    /// launcher (off the UI thread). Each call starts a new process: a restored or split terminal
    /// is a fresh shell, never a replay.
    pub fn new(
        descriptor: BackendDescriptor,
        services: TerminalServices,
        cx: &mut Context<Self>,
    ) -> Self {
        let setting_shell = TerminalSettings::for_cluster(cx, descriptor.cluster()).shell;
        let default_title = descriptor.default_title(setting_shell.as_deref()).into();
        let (mark, follow_mark) = match descriptor.cluster() {
            Some(cluster) => (
                services.launcher().cluster_mark(cluster, cx),
                services.launcher().follow_mark(cluster, cx),
            ),
            None => (None, None),
        };
        let launch = Self::start(&descriptor, &services, cx);
        Self {
            descriptor,
            services,
            phase: Phase::Starting,
            lifecycle: Lifecycle::Connecting,
            element: TerminalElementState::new(),
            focus: cx.focus_handle(),
            default_title,
            process_title: None,
            mark,
            find: Find::default(),
            bell: Bell::default(),
            closed_dir: None,
            _follow_mark: follow_mark,
            launch: Some(launch),
            subscriptions: Vec::new(),
        }
    }

    /// Starts the process `descriptor` describes (off the UI thread) and hands the result to
    /// [`started`](Self::started). The returned task is the launch: keep it in `launch`; dropping
    /// it abandons a start still in flight.
    pub(super) fn start(
        descriptor: &BackendDescriptor,
        services: &TerminalServices,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let started = services.launcher().launch(descriptor, DEFAULT_SIZE, cx);
        cx.spawn(async move |this, cx| {
            let result = started.await;
            // The view may be gone (its tab closed while starting): the backend is dropped here,
            // which ends the process.
            this.update(cx, |this, cx| this.started(result, cx)).ok();
        })
    }

    /// What this terminal was started from.
    pub fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    /// What a copy of this terminal runs, and what its tab saves: the
    /// [`descriptor`](Self::descriptor) with the directory the shell works in now (after a
    /// `cd`), when the backend can tell. A split and a restored tab start there; so does a
    /// reopened one, which keeps the directory the shell was in when its tab closed.
    pub fn live_descriptor(&self, cx: &gpui::App) -> BackendDescriptor {
        let live = match &self.phase {
            Phase::Running(state) => state.read(cx).working_directory(),
            Phase::Closed => self.closed_dir.clone(),
            Phase::Starting | Phase::Failed(_) => None,
        };
        self.descriptor.clone().in_dir_if_known(live)
    }

    /// The session, once the process started.
    pub fn terminal(&self) -> Option<&Entity<TerminalState>> {
        match &self.phase {
            Phase::Running(state) => Some(state),
            _ => None,
        }
    }

    /// Why the process could not start, if it could not.
    pub fn failure(&self) -> Option<&SharedString> {
        match &self.phase {
            Phase::Failed(message) => Some(message),
            _ => None,
        }
    }

    /// Whether a process runs: started and not exited (the tab's dirty dot).
    pub fn is_running(&self, cx: &gpui::App) -> bool {
        self.terminal()
            .is_some_and(|state| state.read(cx).exit_status().is_none())
    }

    /// How the process ended, once it did.
    pub fn exit_status<'a>(&self, cx: &'a gpui::App) -> Option<&'a ExitStatus> {
        match &self.phase {
            Phase::Running(state) => state.read(cx).exit_status(),
            _ => None,
        }
    }

    /// The tab title: what the process set, else the program or pod name.
    pub fn title(&self) -> SharedString {
        self.process_title
            .clone()
            .unwrap_or_else(|| self.default_title.clone())
    }

    /// Shows `mark` (the cluster's colour and read-only flag; `None` draws nothing) on the tab,
    /// redrawing it when it changed. Called by the launcher's
    /// [`follow_mark`](super::TerminalLauncher::follow_mark).
    pub fn set_cluster_mark(&mut self, mark: Option<ClusterMark>, cx: &mut Context<Self>) {
        if self.mark != mark {
            self.mark = mark;
            cx.emit(ItemEvent::UpdateTab);
            cx.notify();
        }
    }

    /// A new terminal running the same thing in the same directory (a fresh process): the split
    /// of this one.
    pub(super) fn duplicate(&self, cx: &mut Context<Self>) -> Entity<Self> {
        let descriptor = self.live_descriptor(cx);
        let services = self.services.clone();
        cx.new(|cx| Self::new(descriptor, services, cx))
    }

    pub(super) fn started(
        &mut self,
        result: OxiResult<Box<dyn TerminalBackend>>,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.phase, Phase::Starting) {
            // Closed meanwhile: dropping the backend ends the process.
            return;
        }
        match result {
            Ok(backend) => {
                let state = cx.new(|cx| TerminalState::new(backend, DEFAULT_SIZE, cx));
                // The state notifies at most once a frame (coalesced); repaint with it, and look
                // an open search's matches up again after new output.
                self.subscriptions.push(cx.observe(&state, |this, _, cx| {
                    this.find_on_terminal_change(cx);
                    cx.notify();
                }));
                self.bell.start_blink(cx);
                self.subscriptions.push(
                    cx.subscribe(&state, |this, _, event: &TerminalEvent, cx| {
                        this.on_terminal_event(event, cx)
                    }),
                );
                self.phase = Phase::Running(state);
                self.signal(Signal::Started);
            }
            Err(error) => {
                // The kind only: a message may name paths of the user's machine.
                tracing::warn!(kind = ?error.kind(), "a terminal could not start");
                self.phase = Phase::Failed(error.to_string().into());
                let failure = if self.descriptor.is_local() {
                    Failure::local_start(&error)
                } else {
                    Failure::from_error(&error)
                };
                self.signal(Signal::StartFailed(failure));
            }
        }
        cx.emit(ItemEvent::UpdateTab);
        cx.notify();
    }

    fn on_terminal_event(&mut self, event: &TerminalEvent, cx: &mut Context<Self>) {
        match event {
            TerminalEvent::TitleChanged(title) => {
                let title = title
                    .as_deref()
                    .map(tab_title)
                    .filter(|title| !title.is_empty())
                    .map(SharedString::from);
                if title != self.process_title {
                    self.process_title = title;
                    cx.emit(ItemEvent::UpdateTab);
                }
            }
            TerminalEvent::Exited(status) => {
                // The dirty dot goes, the banner shows.
                self.signal(Signal::Exited(status.clone()));
                self.stop_input_unless_running(cx);
                cx.emit(ItemEvent::UpdateTab);
                cx.notify();
            }
            TerminalEvent::Error(error) => {
                // Only the kind and the (redacted) message: never output or credentials.
                tracing::debug!(kind = ?error.kind(), "a terminal's transport failed");
                self.signal(Signal::Transport(Failure::from_error(error)));
                self.stop_input_unless_running(cx);
                cx.emit(ItemEvent::UpdateTab);
                cx.notify();
            }
            TerminalEvent::Bell => self.ring_bell(cx),
            TerminalEvent::ClipboardStore(_) | TerminalEvent::ColorRequest(_) => {}
        }
    }

    /// Ends the process and releases the session (the tab closed). Idempotent.
    pub(super) fn shut_down(&mut self, cx: &mut Context<Self>) {
        self.signal(Signal::Close);
        // Cancels a launch still in flight; a backend it already made is dropped with it.
        self.launch = None;
        if let Phase::Running(state) = &self.phase {
            // Read before the kill: the reopen-closed entry may be saved after this.
            self.closed_dir = state.read(cx).working_directory();
            // The kill runs on the tokio bridge; dropping the state below aborts the pump and
            // writer, and the backend goes with the last of them.
            state.update(cx, |state, cx| state.kill(cx)).detach();
        }
        self.phase = Phase::Closed;
        self.subscriptions.clear();
        self.find = Find::default();
        self.bell = Bell::default();
    }
}
