//! Micro benchmark of the catalog: first paint with 20 contexts, the cost of a keystroke in the
//! search field over 2 000 contexts, and a redraw with 500.
//!
//! `cargo run -p oxikube_catalog_ui --features test-support --profile release-fast --example catalog_bench`
//!
//! Runs on GPUI's test platform (no GPU, test text system), so it measures what the catalog adds
//! per frame: reading the (fake) sources, preparing and sorting the rows, filtering, element
//! building and layout of the rows on screen. It is a regression check against the budgets of
//! docs/PERFORMANCE.md (palette: filter 2 000 entries <= 5 ms; first paint of 20 contexts under
//! one frame), not the frame budget itself (that needs `oxikube --perf` in a real window).

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use oxikube_app::{ClusterCatalog, ClusterSessionManager};
use oxikube_catalog_ui::catalog::test_support::{RecordingDispatcher, context, source};
use oxikube_catalog_ui::{CatalogDeps, CatalogView};
use oxikube_ports::ClusterContext;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_ui::root::Root;

fn contexts(n: usize) -> Vec<ClusterContext> {
    (0..n)
        .map(|i| {
            context(&format!(
                "{}-{}-{i}",
                ["prod", "staging", "dev", "qa"][i % 4],
                ["eu", "us", "ap"][i % 3]
            ))
        })
        .collect()
}

struct Window {
    vcx: VisualTestContext,
    view: Entity<CatalogView>,
}

fn open(cx: &mut TestAppContext, n: usize) -> (Window, f64) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
    });
    let source = Arc::new(
        FakeClusterSourcePort::new()
            .with_sources([source()])
            .with_contexts(contexts(n)),
    );
    let clock = Arc::new(FakeClockPort::default());
    let catalog = ClusterCatalog::new(
        source.clone(),
        Arc::new(FakeStatePort::new()),
        clock.clone(),
    );
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        source,
        clock.clone(),
    );
    let deps = CatalogDeps {
        catalog,
        sessions,
        dispatcher: Rc::new(RecordingDispatcher::new()),
        clock,
    };
    let started = Instant::now();
    let mut view = None;
    let window = cx.add_window(|window, cx| {
        let entity = cx.new(|cx| CatalogView::new(deps, window, cx));
        view = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let first_paint_ms = started.elapsed().as_secs_f64() * 1000.0;
    (
        Window {
            vcx,
            view: view.expect("built"),
        },
        first_paint_ms,
    )
}

fn percentile(sorted: &[f64], p: usize) -> f64 {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

fn report(name: &str, mut ms: Vec<f64>) {
    ms.sort_by(|a, b| a.total_cmp(b));
    let mean = ms.iter().sum::<f64>() / ms.len() as f64;
    println!(
        "catalog_bench {name}: {} runs, ms mean {mean:.3} p50 {:.3} p95 {:.3} max {:.3}",
        ms.len(),
        percentile(&ms, 50),
        percentile(&ms, 95),
        ms[ms.len() - 1]
    );
}

fn main() {
    // First paint of 20 contexts: construct, read the sources, sort, draw. A few runs: the first
    // pays for font and icon set-up that every later window shares.
    let mut firsts = Vec::new();
    for _ in 0..15 {
        let mut cx = TestAppContext::single();
        firsts.push(open(&mut cx, 20).1);
    }
    println!(
        "catalog_bench first-paint-20 (cold, first window of the process): {:.3} ms",
        firsts[0]
    );
    report("first-paint-20", firsts);

    // A keystroke in the search field over 2 000 contexts: refilter, then the frame that shows it.
    let mut cx = TestAppContext::single();
    let (mut w, load_ms) = open(&mut cx, 2_000);
    println!("catalog_bench first-paint-2000: {load_ms:.3} ms");
    let queries = [
        "p", "pr", "pro", "prod", "prod-e", "prod-eu", "stag us", "zzz", "", "dev-ap-1",
    ];
    let mut filter_ms = Vec::new();
    let mut frame_ms = Vec::new();
    for round in 0..30 {
        let query = queries[round % queries.len()];
        let view = w.view.clone();
        let started = Instant::now();
        w.vcx
            .update(|window, cx| view.update(cx, |view, cx| view.set_search(query, window, cx)));
        filter_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        w.vcx.update(|window, cx| window.draw(cx).clear(cx));
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("search-keystroke-2000 (filter + notify)", filter_ms);
    report("search-keystroke-2000 (frame after)", frame_ms);

    // A redraw of the 500-context list: virtualised, so it costs what 20 contexts cost.
    let mut cx = TestAppContext::single();
    let (mut w, _) = open(&mut cx, 500);
    let mut frame_ms = Vec::new();
    for _ in 0..200 {
        let started = Instant::now();
        w.vcx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("redraw-500", frame_ms);
}
