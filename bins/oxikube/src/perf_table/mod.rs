//! `oxikube --perf-table <CONTEXT> [--perf-scroll <ROWS>]`: the windowed measurement of the pods
//! table (E07-S09, docs/PERFORMANCE.md "Resource table"). In the library, not the binary, so a
//! `#[gpui::test]` drives it over fakes through the real init path.
//!
//! After the main window opens, this does what a user does, through the same commands: it waits
//! for the catalog to list `CONTEXT`, runs `cluster::Connect` on the command bus (the catalog's
//! Enter: the cluster tab opens and connects), waits for the session, runs `resource::OpenList`
//! for pods (the sidebar's Pods entry), waits for the table to list, and then scrolls it by `ROWS`
//! rows every frame interval, down to the end and back, until the `--perf` session ends. The
//! frame times, feed throughput and notify counts of that run are what `--perf` records.
//!
//! Every step logs to stderr; a step that does not happen within `STEP_DEADLINE` gives up with a
//! message, leaving the app running so the session still records (and the user can take over).

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app_state::AppState;
use crate::startup::window::main_view;
use anyhow::{Context as _, Result, bail};
use gpui::{AnyWindowHandle, App, AsyncApp, Entity, WeakEntity};
use oxikube_app::store::FeedState;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_runtime::{FRAME_INTERVAL, spawn_kube};
use oxikube_workspace::{ClusterCommandRunner, ClusterTab, Workspace};

/// How long each step (catalog, connect, table) may take.
const STEP_DEADLINE: Duration = Duration::from_secs(120);
/// How often a step checks whether it is done.
const POLL: Duration = Duration::from_millis(50);
/// Who the audit log names for the commands this runs.
const WHO: &str = "oxikube --perf-table";

/// What to drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDrive {
    /// The kubeconfig context to connect.
    pub context: String,
    /// Rows scrolled per frame interval (0: open the table and leave it still).
    pub scroll: usize,
    /// Further contexts to connect first, each with its pods table open and left still, so the
    /// run measures several connected clusters (`--perf-also`). `context` is opened last and is
    /// the tab in front.
    pub also: Vec<String>,
}

/// Starts driving `window` (the app's main window). Runs until the window or the app goes.
pub fn start(drive: TableDrive, window: AnyWindowHandle, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        if let Err(err) = run(&drive, window, cx).await {
            eprintln!("oxikube --perf-table: {err:#}; the app keeps recording");
        }
    })
    // Detached on purpose: it lives as long as the app, and ends when the window goes.
    .detach();
}

async fn run(drive: &TableDrive, window: AnyWindowHandle, cx: &mut AsyncApp) -> Result<()> {
    let state = cx.update(|cx| AppState::global(cx));
    let workspace = window.update(cx, |_, window, cx| {
        main_view(window, cx).map(|main| main.read(cx).workspace().clone())
    })?;
    let workspace = workspace.context("the window is not the app's main window")?;

    // The catalog: read like the catalog view reads it.
    let catalog = state.services().catalog.clone();
    let entries = spawn_kube(&*cx, async move { catalog.load().await })
        .await
        .context("the catalog task")?
        .context("reading the kubeconfig catalog")?;
    let find = |context: &str| {
        entries
            .iter()
            .find(|entry| entry.name() == context)
            .map(|entry| entry.id().clone())
            .with_context(|| format!("no context `{context}` in the catalog"))
    };
    let others = drive
        .also
        .iter()
        .map(|context| find(context))
        .collect::<Result<Vec<_>>>()?;
    for (context, other) in drive.also.iter().zip(others) {
        eprintln!("oxikube --perf-table: connecting {context} (not scrolled)");
        let (_, listed) = open_pods(&state, &workspace, window, &other, cx).await?;
        eprintln!(
            "oxikube --perf-table: {context} pods listed {:.0} ms after opening the table",
            listed.as_secs_f64() * 1000.0
        );
    }
    let cluster = find(&drive.context)?;
    eprintln!("oxikube --perf-table: connecting {}", drive.context);
    let (table, listed) = open_pods(&state, &workspace, window, &cluster, cx).await?;
    let rows = cx.update(|cx| table.read(cx).read_rows(cx, |d| d.rows().len()));
    eprintln!(
        "oxikube --perf-table: {rows} pods listed {:.0} ms after opening the table",
        listed.as_secs_f64() * 1000.0
    );
    if drive.scroll == 0 {
        return Ok(());
    }
    eprintln!(
        "oxikube --perf-table: scrolling {} rows every {:.1} ms",
        drive.scroll,
        FRAME_INTERVAL.as_secs_f64() * 1000.0
    );
    // Hold nothing of the window strongly while scrolling: closing it ends the loop.
    let table = table.downgrade();
    drop(workspace);
    scroll(table, drive.scroll, cx).await;
    Ok(())
}

/// What a user does to see a cluster's pods: connects `cluster` (the catalog's Enter), opens its
/// pods table (the sidebar's Pods entry) and waits until it lists. Returns the table and how long
/// the listing took after the table opened.
async fn open_pods(
    state: &Arc<AppState>,
    workspace: &Entity<Workspace>,
    window: AnyWindowHandle,
    cluster: &ClusterId,
    cx: &mut AsyncApp,
) -> Result<(Entity<ResourceTable>, Duration)> {
    run_command(
        state,
        workspace,
        window,
        Command::ClusterConnect {
            cluster: cluster.clone(),
        },
        cx,
    )?;
    wait(cx, "the session to connect", |_| {
        state
            .services()
            .sessions
            .get(cluster)
            .is_some_and(|session| session.is_connected())
    })
    .await?;
    eprintln!("oxikube --perf-table: opening the pods table");
    run_command(
        state,
        workspace,
        window,
        Command::ResourceOpenList {
            cluster: cluster.clone(),
            gvk: Gvk::new("", "v1", "Pod"),
        },
        cx,
    )?;
    let started = Instant::now();
    let mut table = None;
    wait(cx, "the pods table to list", |cx| {
        table = pods_table(workspace, cluster, cx);
        table.as_ref().is_some_and(|table| {
            table.read(cx).read_rows(cx, |d| {
                d.state() == &FeedState::Ready && !d.rows().is_empty()
            })
        })
    })
    .await?;
    Ok((table.context("the pods table")?, started.elapsed()))
}

/// Runs `command` on the window's command bus, as the views dispatch it.
pub(crate) fn run_command(
    state: &Arc<AppState>,
    workspace: &Entity<Workspace>,
    window: AnyWindowHandle,
    command: Command,
    cx: &mut AsyncApp,
) -> Result<()> {
    let bus = state
        .command_bus()
        .cloned()
        .context("the main window has no command bus")?;
    let runner = ClusterCommandRunner::new(bus, WHO, workspace);
    window.update(cx, |_, window, cx| runner.run(command, window, cx))?;
    Ok(())
}

/// The pods table in `cluster`'s tab, once it is open.
fn pods_table(
    workspace: &Entity<Workspace>,
    cluster: &ClusterId,
    cx: &App,
) -> Option<Entity<ResourceTable>> {
    let tab = workspace
        .read(cx)
        .items_of_type::<ClusterTab>()
        .into_iter()
        .find(|tab| tab.read(cx).cluster() == cluster)?;
    let inner = tab.read(cx).workspace().clone();
    inner
        .read(cx)
        .items_of_type::<ResourceTable>()
        .into_iter()
        .find(|table| &*table.read(cx).gvk().kind == "Pod")
}

/// Polls `done` every [`POLL`] until it holds, or fails after [`STEP_DEADLINE`].
pub(crate) async fn wait(
    cx: &mut AsyncApp,
    what: &str,
    mut done: impl FnMut(&App) -> bool,
) -> Result<()> {
    let started = Instant::now();
    loop {
        if cx.update(|cx| done(cx)) {
            return Ok(());
        }
        if started.elapsed() > STEP_DEADLINE {
            bail!("gave up waiting for {what} after {STEP_DEADLINE:?}");
        }
        cx.background_executor().timer(POLL).await;
    }
}

/// Scrolls `table` by `step` rows every frame interval, to the last row and back, until the table
/// or the app goes.
async fn scroll(table: WeakEntity<ResourceTable>, step: usize, cx: &mut AsyncApp) {
    let mut row = 0usize;
    let mut down = true;
    loop {
        cx.background_executor().timer(FRAME_INTERVAL).await;
        let scrolled = table.update(cx, |table, cx| {
            let len = table.read_rows(cx, |d| d.rows().len());
            let last = len.saturating_sub(1);
            row = if down {
                (row + step).min(last)
            } else {
                row.saturating_sub(step)
            };
            if row == last || row == 0 {
                down = row == 0;
            }
            table.table().scroll_to_row(row, cx);
        });
        if scrolled.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
