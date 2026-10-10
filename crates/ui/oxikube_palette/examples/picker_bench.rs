//! Micro benchmark of the picker over 2 000 entries: opening it (construction and first frame), a
//! keystroke (match latency: the query changes until the matches are in, then the frame that
//! shows them), and a redraw. Also the bare match cost of 2 000 entries off any executor.
//!
//! `cargo run -p oxikube_palette --features test-support --profile release-fast --example picker_bench`
//!
//! Runs on GPUI's test platform (no GPU, test text system), so it measures what the picker adds per
//! frame: matching, element building and layout of the visible rows. It is a regression check
//! against docs/PERFORMANCE.md (palette: open <= 1 frame, filter 2 000 entries <= 5 ms,
//! keystroke-to-visible <= 1 frame), not the frame budget itself (that needs `oxikube --perf`).

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::Instant;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use oxikube_palette::Picker;
use oxikube_palette::picker::fuzzy::{StringMatchCandidate, match_strings};
use oxikube_palette::picker::test_support::TestDelegate;
use oxikube_ui::root::Root;

const ENTRIES: usize = 2_000;

struct Open {
    vcx: VisualTestContext,
    picker: Entity<Picker<TestDelegate>>,
}

fn open(cx: &mut TestAppContext) -> (Open, f64) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        cx.set_reduce_motion(true);
    });
    let started = Instant::now();
    let mut picker = None;
    let window = cx.add_window(|window, cx| {
        let entity = cx.new(|cx| Picker::uniform_list(TestDelegate::numbered(ENTRIES), window, cx));
        picker = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    (
        Open {
            vcx,
            picker: picker.expect("built"),
        },
        ms,
    )
}

fn percentile(sorted: &[f64], p: usize) -> f64 {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

fn report(name: &str, mut ms: Vec<f64>) {
    ms.sort_by(|a, b| a.total_cmp(b));
    let mean = ms.iter().sum::<f64>() / ms.len() as f64;
    println!(
        "picker_bench {name}: {} runs, ms mean {mean:.3} p50 {:.3} p95 {:.3} max {:.3}",
        ms.len(),
        percentile(&ms, 50),
        percentile(&ms, 95),
        ms[ms.len() - 1]
    );
}

fn main() {
    // The bare match: what a keystroke costs the background executor.
    let candidates: Vec<_> = (0..ENTRIES)
        .map(|ix| StringMatchCandidate::new(ix, format!("deployment/kube-system/coredns-{ix:04}")))
        .collect();
    let queries = [
        "c", "co", "cor", "core", "kube dns", "sys 19", "zzz", "", "1234", "dep k",
    ];
    let mut match_ms = Vec::new();
    for round in 0..200 {
        let started = Instant::now();
        let found = match_strings(&candidates, queries[round % queries.len()], usize::MAX);
        match_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(found);
    }
    report("match-2000", match_ms);

    // Opening: construction, the empty query's matches and the first frame.
    let mut opens = Vec::new();
    for _ in 0..15 {
        let mut cx = TestAppContext::single();
        opens.push(open(&mut cx).1);
    }
    println!(
        "picker_bench open-2000 (cold, first window of the process): {:.3} ms",
        opens[0]
    );
    report("open-2000", opens);

    // A keystroke: the query changes, the matches arrive (background executor), the frame after.
    let mut cx = TestAppContext::single();
    let (mut w, _) = open(&mut cx);
    let queries = [
        "i", "it", "ite", "item", "item-1", "item-19", "zzz", "", "4", "0 9",
    ];
    let mut keystroke_ms = Vec::new();
    let mut frame_ms = Vec::new();
    for round in 0..50 {
        let query = queries[round % queries.len()];
        let picker = w.picker.clone();
        let started = Instant::now();
        w.vcx
            .update(|window, cx| picker.update(cx, |p, cx| p.set_query(query, window, cx)));
        w.vcx.run_until_parked();
        keystroke_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        w.vcx.update(|window, cx| window.draw(cx).clear(cx));
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("keystroke-2000 (query to matches in)", keystroke_ms);
    report("keystroke-2000 (frame after)", frame_ms);

    // A redraw of the 2 000-entry list: virtualised, so it costs the visible rows only.
    let mut frame_ms = Vec::new();
    for _ in 0..200 {
        let started = Instant::now();
        w.vcx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("redraw-2000", frame_ms);
}
