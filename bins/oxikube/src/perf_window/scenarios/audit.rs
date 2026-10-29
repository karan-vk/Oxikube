//! `audit-log` (E19-S09): the audit viewer over a real SQLite log of 100 000 records. Two phases:
//! `scroll` flings through the table (about 25 rows a refresh, so the table keeps asking for the
//! next keyset page) and reverses at either end; `filters` changes the verb, initiator and outcome
//! filters twice a second, each a new indexed query. The script fails unless the scroll got past
//! the first page and every filter changed the rows.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Result, ensure};
use gpui::{App, Entity};
use oxikube_app::audit::query::{AuditFilters, PAGE_SIZE};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::safety::MutationVerb;
use oxikube_resources_ui::audit::AuditView;
use oxikube_runtime::perf::windowed::Flow;

use super::SETTLE;
use crate::perf_window::driver::{Driver, middle, scroll, tab_workspace};
use crate::perf_window::{AUDIT_RECORDS, MAIN_CONTEXT, PHASE};

/// Pixels a scroll event moves the table: eight times a fast trackpad fling.
const FLING_PX: f32 = 720.0;
/// Refreshes between two filter changes (twice a second at 120 Hz).
const FILTER_EVERY: u64 = 60;

fn loaded(view: &Entity<AuditView>, cx: &App) -> usize {
    view.read(cx).model(cx).len()
}

/// See the [module docs](self).
pub async fn run(driver: &mut Driver<'_>) -> Result<()> {
    let cluster = driver.connect(MAIN_CONTEXT).await?;
    driver.command(Command::AuditOpen { cluster: None })?;
    let workspace = driver.workspace.clone();
    let find = move |cx: &App| {
        tab_workspace(&workspace, &cluster, cx)
            .and_then(|inner| inner.read(cx).items_of_type::<AuditView>().pop())
    };
    let first = find.clone();
    driver
        .wait("the audit viewer's first page", move |cx| {
            first(cx).is_some_and(|view| loaded(&view, cx) >= PAGE_SIZE)
        })
        .await?;
    driver.settle(SETTLE).await;
    let view = driver
        .read(|cx| find(cx))
        .ok_or_else(|| anyhow::anyhow!("the audit viewer closed"))?;

    let furthest = Rc::new(Cell::new(0usize));
    let seen = furthest.clone();
    let handle = view.clone();
    let mut down = true;
    driver
        .phase(
            "scroll",
            Duration::from_secs(20),
            move |step, window, cx| {
                let visible = handle.read(cx).visible_rows(cx);
                let len = loaded(&handle, cx);
                if down && visible.end >= len && len >= AUDIT_RECORDS {
                    down = false;
                } else if !down && visible.start == 0 {
                    down = true;
                }
                seen.set(seen.get().max(visible.start));
                step.input();
                let at = middle(window);
                scroll(window, cx, at, if down { FLING_PX } else { -FLING_PX });
                Ok(Flow::Continue)
            },
        )
        .await?;
    ensure!(
        furthest.get() > PAGE_SIZE,
        "the scroll never got past the first page (first visible row {})",
        furthest.get()
    );

    let handle = view.clone();
    let changed = Rc::new(Cell::new(0u32));
    let count = changed.clone();
    driver
        .phase("filters", PHASE, move |step, _, cx| {
            if step.index.is_multiple_of(FILTER_EVERY) {
                let n = step.index / FILTER_EVERY;
                let filters = match n % 4 {
                    0 => AuditFilters {
                        verb: Some(MutationVerb::Delete),
                        ..AuditFilters::default()
                    },
                    1 => AuditFilters {
                        initiator: Some(Initiator::Agent),
                        ..AuditFilters::default()
                    },
                    2 => AuditFilters {
                        outcome: Some(AuditOutcome::Failed),
                        ..AuditFilters::default()
                    },
                    _ => AuditFilters::default(),
                };
                step.input();
                handle.update(cx, |view, cx| view.set_filters(filters, cx));
                count.set(count.get() + 1);
            }
            Ok(Flow::Continue)
        })
        .await?;
    ensure!(changed.get() > 4, "the filters were not cycled");
    let rows = driver.read(|cx| loaded(&view, cx));
    ensure!(rows > 0, "the last filter matched nothing");
    driver.idle("still", Duration::from_secs(5)).await;
    Ok(())
}
