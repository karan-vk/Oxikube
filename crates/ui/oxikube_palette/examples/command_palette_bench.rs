//! Micro benchmark of the command palette over 2 000 registered commands: classifying them when it
//! opens, opening it (construction and first frame), a keystroke (the query changes, the matches
//! are in, the frame that shows them) and a redraw.
//!
//! `cargo run -p oxikube_palette --features test-support --profile release-fast --example command_palette_bench`
//!
//! Runs on GPUI's test platform (no GPU, test text system): it measures what the palette adds per
//! frame (classifying, matching, element building and layout of the visible rows), a regression
//! check against docs/PERFORMANCE.md (palette: open <= 1 frame, filter 2 000 entries <= 5 ms,
//! keystroke-to-visible <= 1 frame), not the frame budget itself.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::sync::Arc;
use std::time::Instant;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext, WeakEntity};
use oxikube_app::{
    CommandContext, CommandIndex, CommandInfo, CommandTarget, MemoryRecents, RecentsStore,
    Selection,
};
use oxikube_domain::Capabilities;
use oxikube_domain::command::{CommandId, ViewContext};
use oxikube_domain::ids::Gvk;
use oxikube_palette::CommandPalette;
use oxikube_palette::command_palette::{Outbox, PaletteParts, Snapshot};
use oxikube_testkit::commands::fixture_commands;
use oxikube_ui::root::Root;

const COMMANDS: usize = 2_000;

fn index() -> CommandIndex {
    CommandIndex::new(
        fixture_commands(COMMANDS)
            .into_iter()
            .map(|meta| CommandInfo::new(meta, "bench", true)),
    )
    .expect("distinct ids")
}

fn context() -> CommandContext {
    let mut context = CommandContext::new(ViewContext::Table)
        .with_capabilities(Capabilities::all())
        .selecting(Selection::one(Gvk::new("", "v1", "Pod")));
    context.cluster_active = true;
    context
}

fn parts(index: &CommandIndex, recents: Arc<dyn RecentsStore>) -> PaletteParts {
    PaletteParts {
        snapshot: Snapshot::take(index, &context()),
        target: CommandTarget::none(),
        outbox: Outbox::default(),
        recents,
        workspace: WeakEntity::new_invalid(),
    }
}

struct Open {
    vcx: VisualTestContext,
    palette: Entity<CommandPalette>,
}

fn open(cx: &mut TestAppContext, index: &CommandIndex) -> (Open, f64) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        cx.set_reduce_motion(true);
    });
    let recents = Arc::new(MemoryRecents::new());
    for ix in [3, 40, 700] {
        recents.record(CommandId::new(Box::leak(
            format!("pod::Fx{}", ix * 8 + 4).into_boxed_str(),
        )));
    }
    let started = Instant::now();
    let mut palette = None;
    let window = cx.add_window(|window, cx| {
        let entity = cx.new(|cx| CommandPalette::new(parts(index, recents.clone()), window, cx));
        palette = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    (
        Open {
            vcx,
            palette: palette.expect("built"),
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
        "command_palette_bench {name}: {} runs, ms mean {mean:.3} p50 {:.3} p95 {:.3} max {:.3}",
        ms.len(),
        percentile(&ms, 50),
        percentile(&ms, 95),
        ms[ms.len() - 1]
    );
}

fn main() {
    let index = index();

    // Classifying every command against the context (once per open).
    let mut take_ms = Vec::new();
    for _ in 0..100 {
        let started = Instant::now();
        std::hint::black_box(Snapshot::take(&index, &context()));
        take_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("classify-2000 (on open)", take_ms);

    // Opening: classify, construct, list the first frame.
    let mut opens = Vec::new();
    for _ in 0..15 {
        let mut cx = TestAppContext::single();
        opens.push(open(&mut cx, &index).1);
    }
    println!(
        "command_palette_bench open-2000 (cold, first window of the process): {:.3} ms",
        opens[0]
    );
    report("open-2000", opens);

    // A keystroke: the query changes, the matches arrive (background executor above 512
    // candidates), the frame after.
    let mut cx = TestAppContext::single();
    let (mut w, _) = open(&mut cx, &index);
    let queries = [
        "f",
        "fi",
        "fix",
        "fixt",
        "fixture 1",
        "fixture 19",
        "zzz",
        "",
        "4",
        "0 9",
        "pod fixture",
    ];
    let mut keystroke_ms = Vec::new();
    let mut frame_ms = Vec::new();
    for round in 0..55 {
        let query = queries[round % queries.len()];
        let picker = w
            .palette
            .read_with(&w.vcx, |palette, _| palette.picker().clone());
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
