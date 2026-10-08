//! `logs`: a pod's log view (`pod::ViewLogs`, following) while the pod writes 5 000 lines a
//! second: in JSON mode (the default, `logs.json_auto_detect`), with a search typed into it one key
//! at a time (`logs::Find`, then the keys), and as raw text (`logs::ToggleJsonMode`).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use gpui::{App, Entity};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_logs_ui::LogView;
use oxikube_runtime::perf::windowed::Flow;
use oxikube_workspace::Workspace;

use super::{SETTLE, typing};
use crate::perf_window::MAIN_CONTEXT;
use crate::perf_window::driver::{Driver, key, tab_workspace};
use crate::perf_window::world::namespace_name;

/// Each streaming phase.
const STREAM: Duration = Duration::from_secs(10);
/// What is typed into the search: about one line in seven matches `slow`.
const SEARCH: &str = "slow";
/// Refreshes between two keystrokes (about 15 a second).
const KEY_EVERY: u64 = 8;

fn log_view(
    workspace: &Entity<Workspace>,
    cluster: &ClusterId,
    target: &ResourceRef,
    cx: &App,
) -> Option<Entity<LogView>> {
    tab_workspace(workspace, cluster, cx)?
        .read(cx)
        .items_of_type::<LogView>()
        .into_iter()
        .find(|view| view.read(cx).target() == target)
}

/// See the [module docs](self).
pub async fn run(driver: &mut Driver<'_>) -> Result<()> {
    let cluster = driver.connect(MAIN_CONTEXT).await?;
    let target = ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("", "v1", "Pod"),
        namespace_name(0),
        "load-00000",
    );
    driver.command(Command::PodViewLogs {
        target: target.clone(),
        container: None,
        follow: true,
        previous: false,
        tail_lines: None,
    })?;
    let workspace = driver.workspace.clone();
    let mut view = None;
    driver
        .wait("the log view to stream", |cx| {
            view = log_view(&workspace, &cluster, &target, cx);
            view.as_ref()
                .is_some_and(|v| v.read(cx).line_window().line_count() > 0)
        })
        .await?;
    let view = view.context("the log view")?;
    driver.settle(SETTLE).await;
    let json = driver.read(|cx| view.read(cx).options().json);
    ensure!(
        json,
        "JSON mode is not on by default (logs.json_auto_detect)"
    );

    let before = driver.read(|cx| view.read(cx).line_window().next_seq());
    driver
        .phase("stream-json", STREAM, |_, _, _| Ok(Flow::Continue))
        .await?;
    let streamed = driver.read(|cx| view.read(cx).line_window().next_seq()) - before;
    ensure!(
        streamed as f64 > 0.9 * 5_000.0 * STREAM.as_secs_f64(),
        "only {streamed} lines arrived in {STREAM:?}, not 5 000 a second"
    );

    driver.command(Command::LogsFind {
        target: target.clone(),
        pattern: None,
    })?;
    driver.settle(Duration::from_millis(300)).await;
    let most = Rc::new(Cell::new(0));
    let seen = most.clone();
    let searched = view.clone();
    driver
        .phase("type-search", STREAM, move |step, window, cx| {
            seen.set(seen.get().max(searched.read(cx).search_counts().matches));
            if let Some(k) = typing(SEARCH, KEY_EVERY, step.index) {
                step.input();
                key(window, cx, &k)?;
            }
            Ok(Flow::Continue)
        })
        .await?;
    ensure!(
        most.get() > 0,
        "typing `{SEARCH}` into the log search found nothing"
    );
    driver.command(Command::LogsCloseSearch {
        target: target.clone(),
    })?;
    driver.command(Command::LogsToggleJsonMode {
        target: target.clone(),
    })?;
    driver.settle(Duration::from_millis(300)).await;
    let raw = driver.read(|cx| !view.read(cx).options().json);
    ensure!(raw, "logs::ToggleJsonMode did not turn JSON mode off");
    driver
        .phase("stream-raw", STREAM, |_, _, _| Ok(Flow::Continue))
        .await?;
    Ok(())
}
