//! `overview` (E13-S09): the cluster overview, the first screen of a connected cluster, on a
//! 100-node, 3 000-pod cluster churning 1 % every 5 s. Nothing is driven: every refresh is
//! watched, so a frame that pays for the tallies, the view-model rebuild or the cards shows as a
//! long or dropped one. Without a metrics-server or a Prometheus endpoint in the synthetic world,
//! the page draws its `metrics unavailable` state; the charts' repaint cost is the headless
//! `charts` scenario's (E13-S07).

use std::time::Duration;

use anyhow::{Result, ensure};
use oxikube_overview_ui::ClusterOverview;
use oxikube_runtime::perf::windowed::Flow;

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
