//! `terminal`: a shell in the terminal (`terminal::New`, a `LocalPty` on this machine; or with
//! `--perf-exec` `pod::Shell` in a real pod over the kube adapter's exec), driven by what a user
//! types into it:
//!
//! 1. a 50 MB `yes` flood (`yes | head -c 52428800`), watched until it ends;
//! 2. a full-screen redraw at 60 Hz, `htop`-like (every row recoloured and rewritten each frame
//!    by an `awk` script, so it runs in a busybox pod too);
//! 3. the window resized continuously while that redraw goes on (the grid and the process's
//!    size follow).
//!
//! The commands go to the shell as typed text (`TerminalState::input`, what the keyboard sends),
//! in `sh -c '…'` so the user's own shell (fish, zsh) does not change them.

use std::f32::consts::TAU;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use gpui::{App, Entity, px, size};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_runtime::perf::windowed::Flow;
use oxikube_terminal::view::TerminalView;
use oxikube_workspace::{ClusterTab, Workspace};

use super::SETTLE;
use crate::perf_window::driver::Driver;
use crate::perf_window::run::{ExecTarget, WINDOW_SIZE};

/// The flood: 50 MB of `y\n`, then a marker the command line itself does not contain.
const FLOOD: &str = "sh -c 'yes | head -c 52428800; echo flood-$((1+1))-done'\n";
/// Refreshes between two looks at the screen for the flood's end marker.
const CHECK_EVERY: u64 = 30;
/// Longest the flood may take before the scenario gives up.
const FLOOD_DEADLINE: Duration = Duration::from_secs(120);
/// Full-screen frames the redraw script writes (60 a second for 20 s).
const REDRAW_FRAMES: u32 = 1_200;
/// How long the redraw is watched before the resize starts.
const REDRAW: Duration = Duration::from_secs(8);
/// How long the window is resized while the redraw goes on.
const RESIZE: Duration = Duration::from_secs(8);
/// One back-and-forth of the resize drag, in refreshes.
const RESIZE_PERIOD: f32 = 240.0;

/// The `htop`-like redraw: `frames` full screens at about 60 Hz, each row recoloured and rewritten,
/// then a marker.
fn redraw_command(frames: u32) -> String {
    format!(
        "sh -c 's=$(stty size); r=${{s% *}}; c=${{s#* }}; awk -v r=\"$r\" -v c=\"$c\" -v n={frames} \
         '\"'\"'BEGIN {{ pad = sprintf(\"%200s\", \"\"); for (f = 0; f < n; f++) {{ \
         printf \"\\033[H\"; for (i = 1; i < r; i++) {{ \
         name = substr(\"worker-process-\" i \"-\" f pad, 1, c - 40); \
         printf \"\\033[3%dm%6d root %3d.%d%% %7dK S %s\\033[0m\\033[K\\n\", i % 7 + 1, 1000 + i, \
         (f * 7 + i * 13) % 100, (f + i) % 10, (f * 31 + i * 17) % 999999, name }} \
         printf \"\\033[7m frame %d of %d \\033[0m\\033[K\", f, n; fflush(); system(\"sleep 0.0166\") }} }}'\"'\"'; \
         echo; echo redraw-$((1+1))-done'\n"
    )
}

/// See the [module docs](self).
pub async fn run(driver: &mut Driver<'_>, exec: Option<&ExecTarget>) -> Result<()> {
    let terminal = match exec {
        Some(target) => open_pod_shell(driver, target).await?,
        None => open_local_shell(driver).await?,
    };
    let shell = terminal.clone();
    driver
        .wait("the shell to start", move |cx| {
            !screen(&shell, cx).trim().is_empty()
        })
        .await?;
    driver.settle(SETTLE).await;

    type_text(driver, &terminal, FLOOD)?;
    let flood = terminal.clone();
    let done = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = done.clone();
    driver
        .phase("yes-flood", FLOOD_DEADLINE, move |step, _, cx| {
            // Reading the screen copies the grid: four times a second, not on every refresh.
            if step.index.is_multiple_of(CHECK_EVERY) && screen(&flood, cx).contains("flood-2-done")
            {
                seen.set(true);
                return Ok(Flow::Stop);
            }
            Ok(Flow::Continue)
        })
        .await?;
    if !done.get() {
        bail!("the 50 MB flood did not end within {FLOOD_DEADLINE:?}");
    }
    driver.settle(Duration::from_millis(500)).await;

    type_text(driver, &terminal, &redraw_command(REDRAW_FRAMES))?;
    driver
        .phase("redraw-60hz", REDRAW, |_, _, _| Ok(Flow::Continue))
        .await?;
    let redrawn = driver.read(|cx| screen(&terminal, cx));
    if !redrawn.contains("frame ") {
        bail!("the redraw script is not drawing: {redrawn}");
    }
    let (width, height) = WINDOW_SIZE;
    driver
        .phase("resize", RESIZE, move |step, window, _| {
            let phase = (step.index as f32 / RESIZE_PERIOD) * TAU;
            step.input();
            window.resize(size(
                px(width - 300.0 + 300.0 * phase.cos()),
                px(height - 150.0 + 150.0 * phase.sin()),
            ));
            Ok(Flow::Continue)
        })
        .await?;
    driver.update(|window, _| window.resize(size(px(width), px(height))))?;
    Ok(())
}

/// `terminal::New` with no cluster tab shown: a local shell tab of the window.
async fn open_local_shell(driver: &mut Driver<'_>) -> Result<Entity<TerminalView>> {
    driver.command(Command::TerminalNew { cluster: None })?;
    let workspace = driver.workspace.clone();
    let mut terminal = None;
    driver
        .wait("the terminal to open", |cx| {
            terminal = terminals(&workspace, cx).into_iter().next();
            terminal.is_some()
        })
        .await?;
    terminal.context("the terminal")
}

/// `pod::Shell` in `target`, connected from the user's kubeconfig.
async fn open_pod_shell(
    driver: &mut Driver<'_>,
    target: &ExecTarget,
) -> Result<Entity<TerminalView>> {
    let cluster = driver.connect(&target.context).await?;
    driver.command(Command::PodShell {
        target: ResourceRef::namespaced(
            cluster,
            Gvk::new("", "v1", "Pod"),
            target.namespace.as_str(),
            target.pod.as_str(),
        ),
        container: None,
    })?;
    driver.note(format!(
        "the shell runs in pod {}/{} of {} (a real exec)",
        target.namespace, target.pod, target.context
    ));
    let workspace = driver.workspace.clone();
    let mut terminal = None;
    driver
        .wait("the pod terminal to open", |cx| {
            terminal = terminals(&workspace, cx).into_iter().next();
            terminal.is_some()
        })
        .await?;
    terminal.context("the pod terminal")
}

/// Every terminal of the window: its own tabs and those in each cluster tab.
fn terminals(workspace: &Entity<Workspace>, cx: &App) -> Vec<Entity<TerminalView>> {
    let main = workspace.read(cx);
    let mut out = main.items_of_type::<TerminalView>();
    for tab in main.items_of_type::<ClusterTab>() {
        out.extend(
            tab.read(cx)
                .workspace()
                .read(cx)
                .items_of_type::<TerminalView>(),
        );
    }
    out
}

/// Sends `text` to the terminal's process as the keyboard would.
fn type_text(driver: &mut Driver<'_>, terminal: &Entity<TerminalView>, text: &str) -> Result<()> {
    let terminal = terminal.clone();
    let text = text.to_owned();
    driver.update(move |_, cx| {
        let state = terminal
            .read(cx)
            .terminal()
            .cloned()
            .context("the terminal is not running")?;
        state.read(cx).input(text);
        Ok(())
    })?
}

/// The text on the terminal's screen.
fn screen(terminal: &Entity<TerminalView>, cx: &App) -> String {
    let Some(state) = terminal.read(cx).terminal().cloned() else {
        return String::new();
    };
    let snapshot = state.read(cx).snapshot();
    let rows = usize::from(state.read(cx).size().height);
    (0..rows)
        .map(|row| snapshot.row_text(row))
        .collect::<Vec<_>>()
        .join("\n")
}
