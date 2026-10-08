//! `catalog`: the catalog home with 50 contexts; type a search into it one key at a time (its
//! search field, focused as `/` focuses it), erase it and type it again.

use std::cell::Cell;
use std::rc::Rc;

use anyhow::{Context as _, Result, ensure};
use gpui::Entity;
use oxikube_catalog_ui::CatalogView;
use oxikube_runtime::perf::windowed::Flow;

use super::{KEY_EVERY, SETTLE, typing};
use crate::perf_window::PHASE;
use crate::perf_window::driver::{Driver, key};

/// Contexts the catalog lists.
const CONTEXTS: usize = 50;
/// What is typed: narrows the 50 to the ten `perf-listed-4x`, then to a few (the search is fuzzy).
const SEARCH: &str = "listed-42";

/// See the [module docs](self).
pub async fn run(driver: &mut Driver<'_>) -> Result<()> {
    let workspace = driver.workspace.clone();
    let mut catalog: Option<Entity<CatalogView>> = None;
    driver
        .wait("the catalog to list its contexts", |cx| {
            catalog = workspace
                .read(cx)
                .items_of_type::<CatalogView>()
                .into_iter()
                .next();
            catalog
                .as_ref()
                .is_some_and(|view| view.read(cx).model().total() == CONTEXTS)
        })
        .await?;
    let catalog = catalog.context("the catalog view")?;
    let focus = catalog.clone();
    driver.update(|window, cx| focus.update(cx, |view, cx| view.focus_search(window, cx)))?;
    driver.settle(SETTLE).await;
    let fewest = Rc::new(Cell::new(CONTEXTS));
    let seen = fewest.clone();
    driver
        .phase("type-search", PHASE, move |step, window, cx| {
            seen.set(seen.get().min(catalog.read(cx).model().visible_len()));
            if let Some(k) = typing(SEARCH, KEY_EVERY, step.index) {
                step.input();
                key(window, cx, &k)?;
            }
            Ok(Flow::Continue)
        })
        .await?;
    ensure!(
        fewest.get() <= CONTEXTS / 10,
        "typing `{SEARCH}` did not narrow the catalog (at least {} rows throughout)",
        fewest.get()
    );
    Ok(())
}
