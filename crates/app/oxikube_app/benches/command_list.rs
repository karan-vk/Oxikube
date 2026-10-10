//! Micro-benchmark (E11-S01): listing the commands runnable in a context, as the palette does on
//! open and whenever the focused view or selection changes.
//!
//! 2 000 fixture commands (the palette's upper bound: it filters up to 2 000 entries inside a
//! 5 ms budget, `docs/PERFORMANCE.md`) listed in four contexts: a table with one pod selected, the
//! same read-only, a log view and a workspace with no cluster. `cargo bench -p oxikube_app --bench
//! command_list` prints min / median / p95 / max per context; under `cargo test --all-targets` it
//! runs one iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::time::{Duration, Instant};

use oxikube_app::command_bus::{CommandContext, CommandIndex, CommandInfo, Selection};
use oxikube_domain::Capabilities;
use oxikube_domain::command::ViewContext;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::commands::fixture_commands;

const COMMANDS: usize = 2_000;

fn session(view: ViewContext) -> CommandContext {
    let mut ctx = CommandContext::new(view).with_capabilities(Capabilities::all());
    ctx.cluster_active = true;
    ctx
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let bench = std::env::args().any(|a| a == "--bench");
    let samples = if bench { 2_000 } else { 1 };

    let metas = fixture_commands(COMMANDS);
    let index = CommandIndex::new(metas.iter().map(|m| CommandInfo::new(m, "bench", true)))
        .expect("fixture ids are unique");

    let pod = Selection::one(Gvk::new("", "v1", "Pod"));
    let contexts = [
        (
            "table, one pod",
            session(ViewContext::Table).selecting(pod.clone()),
        ),
        (
            "table, one pod, read-only",
            session(ViewContext::Table).selecting(pod).read_only(true),
        ),
        ("log view", session(ViewContext::Logs)),
        (
            "workspace, no cluster",
            CommandContext::new(ViewContext::Workspace),
        ),
    ];

    println!("CommandIndex::list over {COMMANDS} commands ({samples} samples)");
    for (name, ctx) in &contexts {
        let mut times = Vec::with_capacity(samples);
        let mut listed = 0;
        for _ in 0..samples {
            let start = Instant::now();
            listed = black_box(index.list(black_box(ctx))).len();
            times.push(start.elapsed());
        }
        times.sort();
        println!(
            "  {name:<28} {listed:>5} listed  min {:>9.1?}  median {:>9.1?}  p95 {:>9.1?}  max {:>9.1?}",
            times[0],
            percentile(&times, 0.5),
            percentile(&times, 0.95),
            times[times.len() - 1],
        );
    }
}
