//! `overview` (E13-S09): the cluster overview, the first screen of a connected cluster, on a
//! 100-node, 3 000-pod cluster churning 1 % every 5 s. Nothing is driven: every refresh is
//! watched, so a frame that pays for the tallies, the view-model rebuild or the cards shows as a
//! long or dropped one. Without a metrics-server or a Prometheus endpoint in the synthetic world,
//! the page draws its `metrics unavailable` state; the charts' repaint cost is the headless
//! `charts` scenario's (E13-S07).
//!
//! `overview-graphs` (E13-S09-F1): the same page, with a metrics-server and a
//! Prometheus-compatible endpoint in the synthetic world: the usage bars and the top consumers
//! have numbers and the last hour of CPU and memory is drawn as graphs. The scenario fails
//! unless the graphs drew.

use std::time::Duration;

use anyhow::{Result, ensure};
use gpui::App;
use oxikube_overview_ui::ClusterOverview;
use oxikube_overview_ui::overview::HistoryState;
use oxikube_runtime::perf::windowed::Flow;
use oxikube_ui::chart::ChartState;

use super::table::feed_deltas;
use crate::perf_window::driver::{Driver, tab_workspace};
use crate::perf_window::{MAIN_CONTEXT, OVERVIEW_NODES};

/// The cluster's overview: live, and counting every node.
fn counted(overview: &ClusterOverview) -> bool {
    let snapshot = &overview.shown().snapshot;
    overview.is_live()
        && snapshot.nodes_feed.is_listed()
        && snapshot.pods_feed.is_listed()
        && snapshot.nodes.total() == OVERVIEW_NODES
}

/// Whether the overview's usage bars and top consumers have numbers (metrics-server answered).
fn has_usage(overview: &ClusterOverview) -> bool {
    let snapshot = &overview.shown().snapshot;
    snapshot.usage.is_some()
        && !snapshot.top.pods_cpu.is_empty()
        && !snapshot.top.pods_memory.is_empty()
        && !snapshot.top.namespaces_cpu.is_empty()
        && !snapshot.top.namespaces_memory.is_empty()
}

/// Whether the history graphs have their hour of samples and have been painted.
fn graphs_drawn(overview: &ClusterOverview, cx: &App) -> bool {
    let history = overview.history();
    if *history.state() != HistoryState::Ready || history.sparks().is_none() {
        return false;
    }
    [history.cpu_chart(), history.memory_chart()]
        .iter()
        .all(|chart| {
            let chart = chart.read(cx);
            chart.state() == ChartState::Plot && chart.plot_bounds().is_some()
        })
}

/// See the [module docs](self).
pub async fn run(driver: &mut Driver<'_>) -> Result<()> {
    let cluster = driver.connect(MAIN_CONTEXT).await?;
    let workspace = driver.workspace.clone();
    let overview = move |cx: &gpui::App| {
        tab_workspace(&workspace, &cluster, cx)
            .and_then(|inner| inner.read(cx).items_of_type::<ClusterOverview>().pop())
    };
    let first = overview.clone();
    driver
        .wait("the overview counting 100 nodes", move |cx| {
            first(cx).is_some_and(|o| counted(o.read(cx)))
        })
        .await?;
    driver.settle(Duration::from_secs(3)).await;
    let generation = |driver: &mut Driver<'_>| {
        let overview = overview.clone();
        driver.read(move |cx| overview(cx).map(|o| o.read(cx).shown().snapshot.generation))
    };
    let (deltas, before) = (feed_deltas(), generation(driver));
    driver
        .phase("churn", Duration::from_secs(30), |_, _, _| {
            Ok(Flow::Continue)
        })
        .await?;
    ensure!(
        feed_deltas() > deltas,
        "no watch event reached the stores during the churn"
    );
    ensure!(
        generation(driver) > before,
        "the churn never reached the overview's numbers"
    );
    Ok(())
}

/// `overview-graphs` (E13-S09-F1): the cluster overview with live metrics (the usage bars and
/// the top consumers) and a Prometheus-compatible endpoint (the last hour of CPU and memory as
/// graphs), under the churn of the 100-node, 3 000-pod cluster.
pub async fn graphs(driver: &mut Driver<'_>) -> Result<()> {
    let cluster = driver.connect(MAIN_CONTEXT).await?;
    let workspace = driver.workspace.clone();
    let overview = move |cx: &gpui::App| {
        tab_workspace(&workspace, &cluster, cx)
            .and_then(|inner| inner.read(cx).items_of_type::<ClusterOverview>().pop())
    };
    let first = overview.clone();
    driver
        .wait("the overview counting 100 nodes", move |cx| {
            first(cx).is_some_and(|o| counted(o.read(cx)))
        })
        .await?;
    // The usage bars and the top consumers have numbers.
    let usage = overview.clone();
    driver
        .wait("the overview's live usage", move |cx| {
            usage(cx).is_some_and(|o| has_usage(o.read(cx)))
        })
        .await?;
    // The graphs have their hour of samples and are painted.
    let drawn = overview.clone();
    driver
        .wait("the overview's history graphs", move |cx| {
            drawn(cx).is_some_and(|o| graphs_drawn(o.read(cx), cx))
        })
        .await?;
    driver.settle(Duration::from_secs(3)).await;
    let generation = |driver: &mut Driver<'_>| {
        let overview = overview.clone();
        driver.read(move |cx| overview(cx).map(|o| o.read(cx).shown().snapshot.generation))
    };
    let churns = driver.churns();
    let (deltas, before) = (feed_deltas(), generation(driver));
    driver
        .phase("churn", Duration::from_secs(30), |_, _, _| {
            Ok(Flow::Continue)
        })
        .await?;
    // The churn runs on the app's Tokio runtime: without one (the deterministic test platform)
    // the synthetic world does not churn and there is nothing to wait for.
    if churns {
        ensure!(
            feed_deltas() > deltas,
            "no watch event reached the stores during the churn"
        );
        ensure!(
            generation(driver) > before,
            "the churn never reached the overview's numbers"
        );
    }
    // The graphs are still on screen at the end of the churn.
    let after = overview.clone();
    ensure!(
        driver.read(|cx| after(cx).is_some_and(|o| graphs_drawn(o.read(cx), cx))),
        "the history graphs were not on screen at the end of the churn"
    );
    Ok(())
}
