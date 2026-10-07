//! `oxikube --perf-logs <CONTEXT>/<NAMESPACE>/<POD> [--perf-logs-wrap] [--perf-logs-paused]
//! [--perf-logs-workload]`: the windowed measurement of the log viewer (E08-S02,
//! docs/PERFORMANCE.md "Log viewer"). With `--perf-logs-workload` the last part names a
//! Deployment and the view is the merged log of its pods (E08-S04, `workload::ViewLogs`).
//!
//! After the main window opens, this does what a user does, through the same commands: it waits
//! for the catalog to list `CONTEXT`, runs `cluster::Connect` (the cluster tab opens and
//! connects), waits for the session, runs `pod::ViewLogs` for the pod (a pod row's "View Logs"),
//! and, with the flags, `logs::ToggleWrap` and `logs::ToggleAutoscroll` (paused). It then prints
//! the lines received per second every few seconds, while `--perf` records the frame times and
//! notify counts of that run. Point it at a pod that writes fast (a busybox loop printing
//! thousands of lines a second) to measure the streaming budget.
//!
//! Every step logs to stderr; a step that does not happen within the step deadline gives up with
//! a message, leaving the app running so the session still records.

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use gpui::{AnyWindowHandle, App, AsyncApp, Entity};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_logs_ui::LogView;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::{ClusterTab, Workspace};

use crate::app_state::AppState;
use crate::perf_table::{run_command, wait};
use crate::startup::window::main_view;

/// How often the received lines are reported.
const REPORT_EVERY: Duration = Duration::from_secs(5);

/// What to drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogsDrive {
    /// The kubeconfig context to connect.
    pub context: String,
    /// The pod's namespace.
    pub namespace: String,
    /// The pod.
    pub pod: String,
    /// Wrap the lines.
    pub wrap: bool,
    /// Pause autoscroll (the view stays put while lines arrive).
    pub paused: bool,
    /// `pod` names a Deployment: open the merged log of its pods.
    pub workload: bool,
}

impl LogsDrive {
    /// Parses `CONTEXT/NAMESPACE/POD` (the context may itself hold slashes: the last two parts
    /// are the namespace and the pod).
    pub fn parse(value: &str, wrap: bool, paused: bool, workload: bool) -> Option<Self> {
        let mut parts = value.rsplitn(3, '/');
        let pod = parts.next().filter(|s| !s.is_empty())?;
        let namespace = parts.next().filter(|s| !s.is_empty())?;
        let context = parts.next().filter(|s| !s.is_empty())?;
        Some(Self {
            context: context.to_owned(),
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            wrap,
            paused,
            workload,
        })
    }
}

/// Starts driving `window` (the app's main window). Runs until the window or the app goes.
pub fn start(drive: LogsDrive, window: AnyWindowHandle, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        if let Err(err) = run(&drive, window, cx).await {
            eprintln!("oxikube --perf-logs: {err:#}; the app keeps recording");
        }
    })
    // Detached on purpose: it lives as long as the app, and ends when the window goes.
    .detach();
}

async fn run(drive: &LogsDrive, window: AnyWindowHandle, cx: &mut AsyncApp) -> Result<()> {
    let state = cx.update(|cx| AppState::global(cx));
    let workspace = window.update(cx, |_, window, cx| {
        main_view(window, cx).map(|main| main.read(cx).workspace().clone())
    })?;
    let workspace = workspace.context("the window is not the app's main window")?;

    let catalog = state.services().catalog.clone();
    let entries = spawn_kube(&*cx, async move { catalog.load().await })
        .await
        .context("the catalog task")?
        .context("reading the kubeconfig catalog")?;
    let cluster = entries
        .iter()
        .find(|entry| entry.name() == drive.context)
        .map(|entry| entry.id().clone())
        .with_context(|| format!("no context `{}` in the catalog", drive.context))?;
    eprintln!("oxikube --perf-logs: connecting {}", drive.context);
    let connect = Command::ClusterConnect {
        cluster: cluster.clone(),
    };
    run_command(&state, &workspace, window, connect, cx)?;
    wait(cx, "the session to connect", |_| {
        state
            .services()
            .sessions
            .get(&cluster)
            .is_some_and(|session| session.is_connected())
    })
    .await?;

    let gvk = if drive.workload {
        Gvk::new("apps", "v1", "Deployment")
    } else {
        Gvk::new("", "v1", "Pod")
    };
    let target = ResourceRef::namespaced(
        cluster.clone(),
        gvk,
        drive.namespace.as_str(),
        drive.pod.as_str(),
    );
    eprintln!(
        "oxikube --perf-logs: opening the logs of {}/{}",
        drive.namespace, drive.pod
    );
    let open = if drive.workload {
        Command::WorkloadViewLogs {
            target: target.clone(),
            selector: None,
            container: None,
            follow: true,
            tail_lines: None,
        }
    } else {
        Command::PodViewLogs {
            target: target.clone(),
            container: None,
            follow: true,
            previous: false,
            tail_lines: None,
        }
    };
    run_command(&state, &workspace, window, open, cx)?;
    let mut view = None;
    wait(cx, "the log view to stream", |cx| {
        view = log_view(&workspace, &cluster, &target, cx);
        view.as_ref()
            .is_some_and(|view| view.read(cx).line_window().line_count() > 0)
    })
    .await?;
    let view = view.context("the log view")?;
    if drive.wrap {
        let wrap = Command::LogsToggleWrap {
            target: target.clone(),
        };
        run_command(&state, &workspace, window, wrap, cx)?;
    }
    if drive.paused {
        let pause = Command::LogsToggleAutoscroll { target };
        run_command(&state, &workspace, window, pause, cx)?;
    }
    eprintln!(
        "oxikube --perf-logs: streaming (wrap {}, autoscroll {})",
        if drive.wrap { "on" } else { "off" },
        if drive.paused { "paused" } else { "on" }
    );
    let view = view.downgrade();
    drop(workspace);
    let mut last = (Instant::now(), 0u64);
    loop {
        cx.background_executor().timer(REPORT_EVERY).await;
        let Ok(next) = view.read_with(cx, |view, _| view.line_window().next_seq()) else {
            return Ok(());
        };
        let elapsed = last.0.elapsed().as_secs_f64();
        eprintln!(
            "oxikube --perf-logs: {} lines ({:.0} lines/s)",
            next,
            // A view that switched to another session (a replacement pod, E08-S07) counts anew.
            next.saturating_sub(last.1) as f64 / elapsed
        );
        last = (Instant::now(), next);
    }
}

/// The log view of `target` in `cluster`'s tab, once it is open.
fn log_view(
    workspace: &Entity<Workspace>,
    cluster: &ClusterId,
    target: &ResourceRef,
    cx: &App,
) -> Option<Entity<LogView>> {
    let tab = workspace
        .read(cx)
        .items_of_type::<ClusterTab>()
        .into_iter()
        .find(|tab| tab.read(cx).cluster() == cluster)?;
    let inner = tab.read(cx).workspace().clone();
    inner
        .read(cx)
        .items_of_type::<LogView>()
        .into_iter()
        .find(|view| view.read(cx).target() == target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_context_namespace_and_pod() {
        let drive = LogsDrive::parse("kind-oxikube/shop/web-0", true, false, false).unwrap();
        assert_eq!(drive.context, "kind-oxikube");
        assert_eq!(
            (drive.namespace.as_str(), drive.pod.as_str()),
            ("shop", "web-0")
        );
        assert!(drive.wrap && !drive.paused && !drive.workload);
        let arn =
            LogsDrive::parse("arn:aws:eks:eu/cluster/prod/shop/web-0", false, true, true).unwrap();
        assert!(arn.workload);
        assert_eq!(arn.context, "arn:aws:eks:eu/cluster/prod");
        assert!(LogsDrive::parse("shop/web-0", false, false, false).is_none());
        assert!(LogsDrive::parse("ctx//web-0", false, false, false).is_none());
    }
}
