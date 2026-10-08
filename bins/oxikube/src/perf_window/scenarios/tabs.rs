//! `tabs-panes`: three connected clusters, each with its pods table open. Switch cluster tabs
//! (`cluster::NextTab`, `ctrl-tab`) four times a second; resize the window continuously, as when
//! its corner is dragged (every pane and dock is laid out again at each size); resize the front
//! cluster's right dock (the detail drawer) continuously, as when its edge is dragged.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_resources_ui::detail::DetailDrawer;
use oxikube_runtime::perf::windowed::Flow;
use oxikube_ui::Unscaled;
use oxikube_workspace::DockPosition;

use super::{FIRST_POD, SETTLE, drag_phase, drag_size, pods, window_size};
use crate::perf_window::MAIN_CONTEXT;
use crate::perf_window::driver::Driver;
use crate::perf_window::world::namespace_name;

/// Refreshes between two tab switches: a quarter of a second.
const SWITCH_EVERY: u64 = 30;

/// See the [module docs](self).
pub async fn run(driver: &mut Driver<'_>) -> Result<()> {
    let mut front = None;
    for context in [MAIN_CONTEXT, "perf-b", "perf-c"] {
        let cluster = driver.connect(context).await?;
        driver.open_list(&cluster, pods()).await?;
        front = Some(cluster);
    }
    let front = front.context("no cluster")?;
    driver.settle(SETTLE).await;

    let runner = driver.runner();
    let workspace = driver.workspace.clone();
    let shown = Rc::new(RefCell::new(BTreeSet::new()));
    let seen = shown.clone();
    driver
        .phase(
            "switch-tabs",
            Duration::from_secs(10),
            move |step, window, cx| {
                if let Some(item) = workspace.read(cx).active_item(cx) {
                    seen.borrow_mut().insert(item.item_id());
                }
                if step.index.is_multiple_of(SWITCH_EVERY) {
                    step.input();
                    runner.run(Command::ClusterNextTab, window, cx);
                }
                Ok(Flow::Continue)
            },
        )
        .await?;
    ensure!(
        shown.borrow().len() >= 3,
        "cluster::NextTab did not switch the tabs ({} tabs seen in front)",
        shown.borrow().len()
    );

    driver
        .phase(
            "resize-window",
            Duration::from_secs(10),
            move |step, window, _| {
                step.input();
                window.resize(drag_size(step.index));
                Ok(Flow::Continue)
            },
        )
        .await?;
    driver.update(|window, _| window.resize(window_size()))?;
    driver.settle(Duration::from_millis(500)).await;

    // The drawer of a pod in the front cluster's right dock, then its edge dragged.
    driver.command(Command::ResourceOpen {
        target: ResourceRef::namespaced(front.clone(), pods(), namespace_name(0), FIRST_POD),
    })?;
    let inner = driver
        .tab_workspace(&front)
        .context("the front cluster's tab")?;
    let drawer_inner = inner.clone();
    driver
        .wait("the drawer to open", move |cx| {
            drawer_inner
                .read(cx)
                .panel::<DetailDrawer>()
                .is_some_and(|d| d.read(cx).view().is_some())
        })
        .await?;
    driver.settle(SETTLE).await;
    driver
        .phase(
            "resize-dock",
            Duration::from_secs(10),
            move |step, window, cx| {
                let phase = drag_phase(step.index);
                step.input();
                inner.update(cx, |workspace, cx| {
                    workspace.resize_dock(
                        DockPosition::Right,
                        Unscaled(520.0 + 200.0 * phase.sin()),
                        window,
                        cx,
                    );
                });
                Ok(Flow::Continue)
            },
        )
        .await?;
    Ok(())
}
