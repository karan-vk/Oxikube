//! `detail-drawer`: open the detail drawer on a 5 MB ConfigMap the way a user does (its list,
//! then `resource::Open`, the focus in the drawer as after Enter on the row) and cycle its tabs
//! with their keys (`resource_detail::ShowTab`, `1` to `4`): Overview, YAML, Describe, Events.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use gpui::{Action as _, Entity, Focusable as _};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_resources_ui::detail::{DetailDrawer, DetailTab, DetailView, ShowTab};
use oxikube_runtime::perf::windowed::Flow;

use super::SETTLE;
use crate::perf_window::MAIN_CONTEXT;
use crate::perf_window::driver::{Driver, tab_workspace};
use crate::perf_window::world::{BIG_CONFIG_MAP, namespace_name};

/// Refreshes each tab stays on screen: half a second.
const TAB_EVERY: u64 = 60;

/// See the [module docs](self).
pub async fn run(driver: &mut Driver<'_>) -> Result<()> {
    let cluster = driver.connect(MAIN_CONTEXT).await?;
    let config_maps = Gvk::new("", "v1", "ConfigMap");
    driver.open_list(&cluster, config_maps.clone()).await?;
    let target = ResourceRef::namespaced(
        cluster.clone(),
        config_maps,
        namespace_name(0),
        BIG_CONFIG_MAP,
    );
    driver.command(Command::ResourceOpen {
        target: target.clone(),
    })?;
    let workspace = driver.workspace.clone();
    let mut detail: Option<Entity<DetailView>> = None;
    driver
        .wait("the drawer to show the ConfigMap", |cx| {
            detail = tab_workspace(&workspace, &cluster, cx)
                .and_then(|inner| inner.read(cx).panel::<DetailDrawer>())
                .and_then(|drawer| drawer.read(cx).view().cloned())
                .filter(|view| view.read(cx).target() == &target);
            detail
                .as_ref()
                .is_some_and(|view| view.read(cx).model().is_some())
        })
        .await?;
    let detail = detail.context("the detail view")?;
    // Enter on a row moves the focus into the drawer; the tab keys are bound there.
    driver.update(|window, cx| window.focus(&detail.read(cx).focus_handle(cx), cx))?;
    driver.settle(SETTLE).await;
    let shown = Rc::new(RefCell::new(BTreeSet::new()));
    let seen = shown.clone();
    let view = detail.clone();
    driver
        .phase(
            "cycle-tabs",
            Duration::from_secs(20),
            move |step, window, cx| {
                seen.borrow_mut()
                    .insert(format!("{:?}", view.read(cx).tab()));
                if step.index.is_multiple_of(TAB_EVERY) {
                    let index = u8::try_from((step.index / TAB_EVERY) % 4 + 1).unwrap_or(1);
                    step.input();
                    window.dispatch_action(ShowTab { index }.boxed_clone(), cx);
                }
                Ok(Flow::Continue)
            },
        )
        .await?;
    let shown = shown.borrow();
    ensure!(
        DetailTab::ALL
            .iter()
            .all(|tab| shown.contains(&format!("{tab:?}"))),
        "the tab keys did not reach the drawer (tabs shown: {shown:?})"
    );
    Ok(())
}
