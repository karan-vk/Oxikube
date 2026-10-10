//! [`Driver`]: what a windowed scenario's script uses. Setup goes through the same commands a
//! user's clicks send (`cluster::Connect`, `resource::OpenList`, ...); scripted phases run a step
//! on every display refresh ([`windowed::drive`]) and dispatch input to the window the way the
//! platform does ([`scroll`], [`key`], [`Driver::runner`] for commands).

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use gpui::{
    AnyWindowHandle, App, AsyncApp, Entity, Keystroke, Modifiers, Pixels, PlatformInput, Point,
    ScrollDelta, ScrollWheelEvent, TouchPhase, Window, point, px,
};
use oxikube_app::store::FeedState;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_runtime::perf::windowed::{self, Flow, Meter, Step};
use oxikube_runtime::spawn_kube;
use oxikube_workspace::{ClusterCommandRunner, ClusterTab, Workspace};

use crate::app_state::AppState;

/// How long a setup step (connect, list) may take.
const STEP_DEADLINE: Duration = Duration::from_secs(120);
/// How often a setup step checks whether it is done.
const POLL: Duration = Duration::from_millis(20);
/// Who the audit log names for the commands scenarios run.
pub const WHO: &str = "oxikube --perf-scenario-window";

/// See the [module docs](self).
pub struct Driver<'a> {
    /// The app's async context.
    pub cx: &'a mut AsyncApp,
    /// The main window.
    pub window: AnyWindowHandle,
    /// The scenario's measurements.
    pub meter: Meter,
    /// The app state.
    pub state: Arc<AppState>,
    /// The window's workspace.
    pub workspace: Entity<Workspace>,
    runner: ClusterCommandRunner,
    notes: Vec<String>,
}

impl<'a> Driver<'a> {
    /// A driver of `window` (the app's main window, `workspace` its workspace).
    pub fn new(
        cx: &'a mut AsyncApp,
        window: AnyWindowHandle,
        meter: Meter,
        workspace: Entity<Workspace>,
    ) -> Result<Self> {
        let state = cx.update(|cx| AppState::global(cx));
        let bus = state
            .command_bus()
            .cloned()
            .context("the main window has no command bus")?;
        let runner =
            ClusterCommandRunner::new(bus, state.services().sessions.clone(), WHO, &workspace);
        Ok(Self {
            cx,
            window,
            meter,
            state,
            workspace,
            runner,
            notes: Vec::new(),
        })
    }

    /// The command runner, for steps that dispatch commands: the same runner the views use.
    pub fn runner(&self) -> ClusterCommandRunner {
        self.runner.clone()
    }

    /// Adds a fact the summary should carry.
    pub fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    /// The notes so far.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// Runs `command` as the views dispatch it.
    pub fn command(&mut self, command: Command) -> Result<()> {
        let runner = self.runner.clone();
        self.window
            .update(self.cx, |_, window, cx| runner.run(command, window, cx))
            .context("the window is gone")
    }

    /// Runs `f` on the window.
    pub fn update<R>(&mut self, f: impl FnOnce(&mut Window, &mut App) -> R) -> Result<R> {
        self.window
            .update(self.cx, |_, window, cx| f(window, cx))
            .context("the window is gone")
    }

    /// Reads the app.
    pub fn read<R>(&mut self, f: impl FnOnce(&App) -> R) -> R {
        self.cx.update(|cx| f(cx))
    }

    /// Waits until `done` holds, polling every [`POLL`]; fails after [`STEP_DEADLINE`].
    pub async fn wait(&mut self, what: &str, mut done: impl FnMut(&App) -> bool) -> Result<()> {
        let started = Instant::now();
        loop {
            if self.cx.update(|cx| done(cx)) {
                return Ok(());
            }
            if started.elapsed() > STEP_DEADLINE {
                bail!("gave up waiting for {what} after {STEP_DEADLINE:?}");
            }
            self.cx.background_executor().timer(POLL).await;
        }
    }

    /// Lets `duration` pass (setup: a list landing, a view settling).
    pub async fn settle(&mut self, duration: Duration) {
        self.cx.background_executor().timer(scaled(duration)).await;
    }

    /// A scripted phase: `step` on every display refresh for `duration` (see `windowed::drive`).
    pub async fn phase(
        &mut self,
        name: &str,
        duration: Duration,
        step: impl FnMut(&Step<'_>, &mut Window, &mut App) -> Result<Flow> + 'static,
    ) -> Result<()> {
        eprintln!("oxikube --perf-scenario-window: phase {name} ({duration:?})");
        #[cfg(test)]
        let (duration, step) = refresh_bound(duration, step);
        windowed::drive(self.cx, self.window, &self.meter, name, duration, step).await
    }

    /// An idle phase: nothing driven for `duration`.
    pub async fn idle(&mut self, name: &str, duration: Duration) {
        eprintln!("oxikube --perf-scenario-window: phase {name} ({duration:?}, idle)");
        windowed::idle(self.cx, self.window, &self.meter, name, scaled(duration)).await;
    }

    /// Connects `context` as the catalog's Enter does and waits for the session.
    pub async fn connect(&mut self, context: &str) -> Result<ClusterId> {
        let catalog = self.state.services().catalog.clone();
        let entries = spawn_kube(&*self.cx, async move { catalog.load().await })
            .await
            .context("the catalog task")?
            .context("reading the catalog")?;
        let cluster = entries
            .iter()
            .find(|entry| entry.name() == context)
            .map(|entry| entry.id().clone())
            .with_context(|| format!("no context `{context}` in the catalog"))?;
        eprintln!("oxikube --perf-scenario-window: connecting {context}");
        self.command(Command::ClusterConnect {
            cluster: cluster.clone(),
        })?;
        let sessions = self.state.services().sessions.clone();
        let id = cluster.clone();
        self.wait("the session to connect", move |_| {
            sessions.get(&id).is_some_and(|s| s.is_connected())
        })
        .await?;
        Ok(cluster)
    }

    /// The inner workspace of `cluster`'s tab.
    pub fn tab_workspace(&mut self, cluster: &ClusterId) -> Option<Entity<Workspace>> {
        let workspace = self.workspace.clone();
        self.read(|cx| tab_workspace(&workspace, cluster, cx))
    }

    /// Opens `cluster`'s list of `gvk` as the sidebar does (`resource::OpenList`) and waits until
    /// it lists.
    pub async fn open_list(
        &mut self,
        cluster: &ClusterId,
        gvk: Gvk,
    ) -> Result<Entity<ResourceTable>> {
        self.command(Command::ResourceOpenList {
            cluster: cluster.clone(),
            gvk: gvk.clone(),
        })?;
        let started = Instant::now();
        let mut table = None;
        let workspace = self.workspace.clone();
        let kind = gvk.kind.clone();
        self.wait("the table to list", |cx| {
            table = tab_workspace(&workspace, cluster, cx).and_then(|inner| {
                inner
                    .read(cx)
                    .items_of_type::<ResourceTable>()
                    .into_iter()
                    .find(|t| t.read(cx).gvk().kind == kind)
            });
            table.as_ref().is_some_and(|t| {
                t.read(cx).read_rows(cx, |d| {
                    d.state() == &FeedState::Ready && !d.rows().is_empty()
                })
            })
        })
        .await?;
        let table = table.context("the table")?;
        let rows = self.read(|cx| table.read(cx).read_rows(cx, |d| d.rows().len()));
        eprintln!(
            "oxikube --perf-scenario-window: {rows} {} listed after {:.0} ms",
            gvk.kind,
            started.elapsed().as_secs_f64() * 1000.0
        );
        Ok(table)
    }
}

/// Percent of its length a phase lasts in this library's tests, which run the scripts on GPUI's
/// test platform frame by frame (100 everywhere else: a run is never shortened). There a phase is
/// that share of its refreshes at 120 Hz, however long the test machine takes to draw them.
#[cfg(test)]
pub(crate) static TEST_TIME_PERCENT: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(100);

fn scaled(duration: Duration) -> Duration {
    #[cfg(test)]
    {
        let percent = TEST_TIME_PERCENT.load(std::sync::atomic::Ordering::Relaxed);
        duration * percent / 100
    }
    #[cfg(not(test))]
    duration
}

/// In tests: `duration` as a count of 120 Hz refreshes (scaled), the step stopped after them.
#[cfg(test)]
fn refresh_bound(
    duration: Duration,
    mut step: impl FnMut(&Step<'_>, &mut Window, &mut App) -> Result<Flow> + 'static,
) -> (
    Duration,
    impl FnMut(&Step<'_>, &mut Window, &mut App) -> Result<Flow> + 'static,
) {
    let refreshes = (scaled(duration).as_secs_f64() * 120.0) as u64;
    let bounded = move |s: &Step<'_>, window: &mut Window, cx: &mut App| {
        if s.index >= refreshes {
            return Ok(Flow::Stop);
        }
        step(s, window, cx)
    };
    (Duration::from_secs(3600), bounded)
}

/// The inner workspace of `cluster`'s tab in `workspace`.
pub fn tab_workspace(
    workspace: &Entity<Workspace>,
    cluster: &ClusterId,
    cx: &App,
) -> Option<Entity<Workspace>> {
    let tab = workspace
        .read(cx)
        .items_of_type::<ClusterTab>()
        .into_iter()
        .find(|tab| tab.read(cx).cluster() == cluster)?;
    Some(tab.read(cx).workspace().clone())
}

/// Dispatches a trackpad scroll of `dy` pixels (positive: content moves up, as when scrolling
/// down) at `position`, as the platform delivers one.
pub fn scroll(window: &mut Window, cx: &mut App, position: Point<Pixels>, dy: f32) {
    window.dispatch_event(
        PlatformInput::ScrollWheel(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(-dy))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        }),
        cx,
    );
}

/// Dispatches keystroke `key` (`"a"`, `"backspace"`, `"enter"`, `"cmd-f"`) as the platform
/// delivers a key press: key bindings first, then the focused input handler.
pub fn key(window: &mut Window, cx: &mut App, key: &str) -> Result<()> {
    let keystroke = Keystroke::parse(key).with_context(|| format!("keystroke `{key}`"))?;
    window.dispatch_keystroke(keystroke, cx);
    Ok(())
}

/// The point in the middle of the window's content.
pub fn middle(window: &Window) -> Point<Pixels> {
    let size = window.viewport_size();
    point(size.width / 2., size.height / 2.)
}
