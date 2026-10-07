//! `#[gpui::test]` suite for the terminal tab (E09-S07): `TerminalView` as a workspace item (title,
//! dirty while running, closing ends the process), moving it between panes and the bottom dock
//! without recreating it, layout persistence (the descriptor only, a fresh process on restore),
//! and the `terminal::New` / `Split` / `Close` commands. Over `FakeTerminalBackend` on the
//! deterministic runtime: no process, no OS thread.

mod commands;
mod dock;
mod find;
mod item;
mod leak;
mod lifecycle;
mod persist;
mod pod;
mod settings;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::stream::BoxStream;
use gpui::{AppContext as _, Entity, Task, TestAppContext, VisualTestContext};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{BackendEvent, TerminalBackend, TerminalSize};
use oxikube_runtime::FRAME_INTERVAL;
use oxikube_terminal::view::{
    BackendDescriptor, Launch, TerminalLauncher, TerminalServices, TerminalView,
};
use oxikube_testkit::fakes::FakeTerminalBackend;
use oxikube_workspace::{ClusterMark, CommandDispatcher, Workspace};

/// Starts a `FakeTerminalBackend` for every launch and remembers what it was asked.
#[derive(Default)]
struct FakeLauncher {
    launches: RefCell<Vec<BackendDescriptor>>,
    backends: RefCell<Vec<FakeTerminalBackend>>,
    fail_next: RefCell<Option<OxiError>>,
    mark: Option<ClusterMark>,
    /// Never finish starting: the tab stays in its "Starting" state.
    hang: bool,
    /// Counts the backends that are alive (started and not dropped yet), when set.
    alive: Option<Arc<AtomicUsize>>,
}

/// A backend that counts itself while it is alive: what is left of a closed terminal's process.
struct Probed {
    inner: FakeTerminalBackend,
    alive: Arc<AtomicUsize>,
}

impl Probed {
    fn new(inner: FakeTerminalBackend, alive: Arc<AtomicUsize>) -> Self {
        alive.fetch_add(1, Ordering::SeqCst);
        Self { inner, alive }
    }
}

impl Drop for Probed {
    fn drop(&mut self) {
        self.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl TerminalBackend for Probed {
    async fn write(&self, bytes: &[u8]) -> OxiResult<()> {
        self.inner.write(bytes).await
    }

    async fn resize(&self, size: TerminalSize) -> OxiResult<()> {
        self.inner.resize(size).await
    }

    fn output_stream(&self) -> BoxStream<'static, BackendEvent> {
        self.inner.output_stream()
    }

    async fn kill(&self) -> OxiResult<()> {
        self.inner.kill().await
    }

    fn working_directory(&self) -> Option<PathBuf> {
        self.inner.working_directory()
    }
}

impl TerminalLauncher for FakeLauncher {
    fn launch(
        &self,
        descriptor: &BackendDescriptor,
        _: TerminalSize,
        cx: &mut gpui::App,
    ) -> Launch {
        self.launches.borrow_mut().push(descriptor.clone());
        if self.hang {
            return cx.spawn(async move |_| std::future::pending().await);
        }
        if let Some(error) = self.fail_next.borrow_mut().take() {
            return Task::ready(Err(error));
        }
        let backend = FakeTerminalBackend::silent();
        self.backends.borrow_mut().push(backend.clone());
        match &self.alive {
            Some(alive) => Task::ready(Ok(Box::new(Probed::new(backend, alive.clone())))),
            None => Task::ready(Ok(Box::new(backend))),
        }
    }

    fn cluster_mark(&self, _: &ClusterId, _: &gpui::App) -> Option<ClusterMark> {
        self.mark
    }
}

/// Records the commands the views send.
#[derive(Default)]
struct Recorder(RefCell<Vec<Command>>);

impl CommandDispatcher for Recorder {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.0.borrow_mut().push(command);
    }
}

/// A workspace window with the terminal crate registered and its services installed.
struct Harness {
    ws: Entity<Workspace>,
    vcx: VisualTestContext,
    launcher: Rc<FakeLauncher>,
    recorder: Rc<Recorder>,
    services: TerminalServices,
}

fn harness(cx: &mut TestAppContext) -> Harness {
    harness_with(cx, FakeLauncher::default())
}

fn harness_with(cx: &mut TestAppContext, launcher: FakeLauncher) -> Harness {
    cx.update(oxikube_runtime::init_deterministic);
    let (ws, vcx) = oxikube_workspace::test_support::open_workspace(cx);
    let launcher = Rc::new(launcher);
    let recorder = Rc::new(Recorder::default());
    let services = TerminalServices::new(launcher.clone()).with_dispatcher(recorder.clone());
    let installed = services.clone();
    cx.update(|cx| {
        oxikube_terminal::init(cx);
        oxikube_terminal::view::install(installed, cx);
    });
    Harness {
        ws,
        vcx,
        launcher,
        recorder,
        services,
    }
}

fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("kind-dev"))
}

impl Harness {
    /// Builds a terminal for `descriptor` (not opened anywhere yet).
    fn terminal(&mut self, descriptor: BackendDescriptor) -> Entity<TerminalView> {
        let services = self.services.clone();
        let view = self
            .vcx
            .update(|_, cx| cx.new(|cx| TerminalView::new(descriptor, services, cx)));
        self.vcx.run_until_parked();
        view
    }

    /// Builds a terminal for `descriptor` and opens it in the active pane.
    fn open(&mut self, descriptor: BackendDescriptor) -> Entity<TerminalView> {
        let view = self.terminal(descriptor);
        let ws = self.ws.clone();
        let opened = view.clone();
        self.vcx
            .update(|window, cx| ws.update(cx, |ws, cx| ws.open_item(opened, window, cx)));
        self.vcx.run_until_parked();
        view
    }

    /// Lets queued work run and one frame pass (the coalesced notify), then draws.
    fn frame(&mut self) {
        self.vcx.run_until_parked();
        self.vcx.executor().advance_clock(FRAME_INTERVAL);
        self.vcx.run_until_parked();
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// The `n`th backend the launcher started.
    fn backend(&self, n: usize) -> FakeTerminalBackend {
        self.launcher.backends.borrow()[n].clone()
    }

    fn launches(&self) -> Vec<BackendDescriptor> {
        self.launcher.launches.borrow().clone()
    }

    /// The text of `row` of `view`'s screen.
    fn row(&mut self, view: &Entity<TerminalView>, row: usize) -> String {
        self.vcx.update(|_, cx| {
            let state = view.read(cx).terminal().expect("running").clone();
            state.read(cx).snapshot().row_text(row)
        })
    }

    /// Whether `selector` is drawn.
    fn drawn(&mut self, selector: &'static str) -> bool {
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
        self.vcx.debug_bounds(selector).is_some()
    }
}
