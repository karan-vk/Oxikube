//! The scenarios around a cluster's pods table: scroll it, filter it, switch its namespace, switch
//! the theme over it, watch the sidebar's badges under churn, and leave two clusters idle.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Result, bail, ensure};
use gpui::{App, Entity, UpdateGlobal as _};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_runtime::perf::windowed::Flow;
use oxikube_settings::SettingsStore;
use oxikube_theme::ActiveTheme;

use super::{SETTLE, typing};
use crate::perf_window::driver::{Driver, key, middle, scroll};
use crate::perf_window::world::namespace_name;
use crate::perf_window::{MAIN_CONTEXT, PHASE, PODS};

/// Pixels a scroll event moves the table: a fast trackpad fling (about three rows a refresh).
const FLING_PX: f32 = 90.0;
/// What the filter scenario types: about one row in a hundred matches it fully.
const FILTER_TEXT: &str = "load-012";
/// Refreshes between two keystrokes: about 15 characters a second at 120 Hz, a fast typist.
const KEY_EVERY: u64 = 8;

fn pods() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn rows(table: &Entity<ResourceTable>, cx: &App) -> usize {
    table.read(cx).read_rows(cx, |d| d.rows().len())
}

/// Connects the main cluster and opens its pods table, listed.
async fn main_table(driver: &mut Driver<'_>) -> Result<(ClusterId, Entity<ResourceTable>)> {
    let cluster = driver.connect(MAIN_CONTEXT).await?;
    let table = driver.open_list(&cluster, pods()).await?;
    driver.settle(SETTLE).await;
    Ok((cluster, table))
}

/// `pods-table`: fling through the 10 000 pods while 1 % of them churn every 5 s, reversing at
/// either end; then leave the table still with the churn going.
pub async fn pods_table(driver: &mut Driver<'_>) -> Result<()> {
    let (_, table) = main_table(driver).await?;
    let first = driver.read(|cx| table.read(cx).table().visible_rows(cx));
    let seen = Rc::new(Cell::new(first.start));
    let furthest = seen.clone();
    let mut down = true;
    let handle = table.clone();
    driver
        .phase(
            "scroll",
            Duration::from_secs(20),
            move |step, window, cx| {
                let visible = handle.read(cx).table().visible_rows(cx);
                let len = rows(&handle, cx);
                if down && visible.end >= len {
                    down = false;
                } else if !down && visible.start == 0 {
                    down = true;
                }
                furthest.set(furthest.get().max(visible.start));
                step.input();
                let at = middle(window);
                scroll(window, cx, at, if down { FLING_PX } else { -FLING_PX });
                Ok(Flow::Continue)
            },
        )
        .await?;
    ensure!(
        seen.get() > first.start,
        "the scroll events did not scroll the table (first visible row stayed near {})",
        first.start
    );
    driver.idle("still", Duration::from_secs(10)).await;
    Ok(())
}

/// `table-filter`: focus the table's filter (`table::FocusFilter`, `/`) and type into it, one
/// keystroke every [`KEY_EVERY`] refreshes, erasing and typing again.
pub async fn filter(driver: &mut Driver<'_>) -> Result<()> {
    let (cluster, table) = main_table(driver).await?;
    driver.command(Command::TableFocusFilter {
        cluster,
        gvk: pods(),
    })?;
    driver.settle(Duration::from_millis(300)).await;
    let fewest = Rc::new(Cell::new(PODS));
    let seen = fewest.clone();
    let handle = table.clone();
    driver
        .phase("type-filter", PHASE, move |step, window, cx| {
            seen.set(seen.get().min(rows(&handle, cx)));
            if let Some(k) = typing(FILTER_TEXT, KEY_EVERY, step.index) {
                step.input();
                key(window, cx, &k)?;
            }
            Ok(Flow::Continue)
        })
        .await?;
    ensure!(
        fewest.get() < PODS / 10,
        "typing `{FILTER_TEXT}` did not filter the table (at least {} rows throughout)",
        fewest.get()
    );
    Ok(())
}

/// `namespaces`: switch the cluster's namespace (`namespace::Select`) four times a second through
/// its 8 namespaces and back to all, under churn.
pub async fn namespaces(driver: &mut Driver<'_>) -> Result<()> {
    let (cluster, table) = main_table(driver).await?;
    let runner = driver.runner();
    let fewest = Rc::new(Cell::new(PODS));
    let seen = fewest.clone();
    let handle = table.clone();
    driver
        .phase("switch-namespace", PHASE, move |step, window, cx| {
            seen.set(seen.get().min(rows(&handle, cx)));
            if step.index.is_multiple_of(30) {
                let n = (step.index / 30) % 9;
                let namespaces = if n == 8 {
                    Vec::new()
                } else {
                    vec![namespace_name(usize::try_from(n).unwrap_or(0))]
                };
                step.input();
                runner.run(
                    Command::NamespaceSelect {
                        cluster: cluster.clone(),
                        namespaces,
                    },
                    window,
                    cx,
                );
            }
            Ok(Flow::Continue)
        })
        .await?;
    ensure!(
        fewest.get() < PODS / 4,
        "switching namespace did not narrow the table (at least {} rows throughout)",
        fewest.get()
    );
    Ok(())
}

/// `theme`: switch between One Light and One Dark twice a second over the pods table, through the
/// settings store (the path a `settings.json` hot reload takes).
pub async fn theme(driver: &mut Driver<'_>) -> Result<()> {
    main_table(driver).await?;
    let names = Rc::new(std::cell::RefCell::new(std::collections::BTreeSet::new()));
    let seen = names.clone();
    driver
        .phase("switch-theme", PHASE, move |step, _, cx| {
            seen.borrow_mut().insert(ActiveTheme::get(cx).name.clone());
            if step.index.is_multiple_of(60) {
                let theme = if (step.index / 60) % 2 == 0 {
                    "One Light"
                } else {
                    "One Dark"
                };
                step.input();
                SettingsStore::update_global(cx, |store, _| {
                    store.set_user_settings(&format!(r#"{{"theme": "{theme}"}}"#))
                })?;
            }
            Ok(Flow::Continue)
        })
        .await?;
    let seen = names.borrow().len();
    if seen < 2 {
        bail!("the theme never changed ({seen} theme seen)");
    }
    Ok(())
}

/// `sidebar`: the cluster's first screen (the Workloads overview) with the sidebar's count badges,
/// nothing driven but the churn: every refresh is watched, so a churn frame that runs long shows
/// as a dropped one.
pub async fn sidebar(driver: &mut Driver<'_>) -> Result<()> {
    let cluster = driver.connect(MAIN_CONTEXT).await?;
    let workspace = driver.workspace.clone();
    driver
        .wait("the cluster's first screen", |cx| {
            crate::perf_window::driver::tab_workspace(&workspace, &cluster, cx)
                .is_some_and(|inner| inner.read(cx).items().next().is_some())
        })
        .await?;
    driver.settle(Duration::from_secs(3)).await;
    let before = oxikube_runtime::perf::global().map_or(0, |r| r.feed_deltas());
    driver
        .phase("churn", Duration::from_secs(30), |_, _, _| {
            Ok(Flow::Continue)
        })
        .await?;
    let applied = oxikube_runtime::perf::global().map_or(0, |r| r.feed_deltas()) - before;
    ensure!(
        applied > 0,
        "no watch event reached the stores during the churn"
    );
    Ok(())
}

/// `idle`: two clusters connected, each with its pods table open, nothing moving.
pub async fn idle(driver: &mut Driver<'_>) -> Result<()> {
    for context in [MAIN_CONTEXT, "perf-b"] {
        let cluster = driver.connect(context).await?;
        driver.open_list(&cluster, pods()).await?;
    }
    driver.settle(Duration::from_secs(5)).await;
    driver.idle("idle", Duration::from_secs(30)).await;
    Ok(())
}
