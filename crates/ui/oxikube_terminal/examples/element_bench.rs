//! Non-gating paint benchmark of `TerminalElement` (E09-S05).
//!
//! `cargo run --profile release-fast -p oxikube_terminal --example element_bench`
//!
//! Headless GPUI (the host's real text system and headless renderer, no present), an 80 x 24 and
//! a 240 x 60 grid, four workloads fed through `FakeTerminalBackend` -> `TerminalState` one wave
//! per frame:
//!
//! - `idle`: nothing new (a repaint for a hover or a blink);
//! - `yes`: 4 KiB of `y\n` per frame (the screen scrolls, every line the same);
//! - `ls`: 8 new coloured lines per frame (the screen scrolls, every line different);
//! - `htop`: every row redrawn with new numbers per frame (cursor addressing, SGR).
//!
//! Per workload: the frame time (the frame tick firing the coalesced notify, then layout +
//! prepaint + paint of the whole window, every reshape included) p50 / p95 / max, a repaint with
//! nothing new (p50), allocations per frame (median, counting global allocator; most are GPUI's
//! own per-frame bookkeeping), the row cache's hit rate and the runs shaped per frame. Headless
//! numbers are CPU and paint preparation only: no GPU time.
#![allow(clippy::print_stdout)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, FocusHandle, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div, px, size,
};
use oxikube_ports::TerminalSize;
use oxikube_runtime::FRAME_INTERVAL;
use oxikube_terminal::element::CellMetrics;
use oxikube_terminal::{TerminalElement, TerminalElementState, TerminalFont, TerminalState};
use oxikube_testkit::fakes::FakeTerminalBackend;
use oxikube_testkit::headless::headless_context;

struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

// SAFETY: forwards to the system allocator; only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const WARMUP: usize = 30;
const FRAMES: usize = 300;

fn font() -> TerminalFont {
    TerminalFont {
        family: TerminalFont::platform_family().into(),
        size: px(13.),
        line_height: 1.3,
    }
}

struct Host {
    terminal: Entity<TerminalState>,
    state: TerminalElementState,
    focus: FocusHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(TerminalElement::new(&self.terminal, &self.state, &self.focus).font(font()))
    }
}

fn wave(workload: &str, frame: usize, rows: usize, columns: usize) -> String {
    match workload {
        "idle" => String::new(),
        "yes" => "y\r\n".repeat(1366),
        "ls" => (0..8)
            .map(|i| {
                let n = frame * 8 + i;
                format!(
                    "\x1b[01;34mdir-{n}\x1b[0m  \x1b[01;32mexec-{n}\x1b[0m  \x1b[38;5;208mfile-{n}.rs\x1b[0m  {n:>8} bytes\r\n"
                )
            })
            .collect(),
        "htop" => {
            let mut out = String::from("\x1b[H");
            for row in 1..=rows {
                let line = format!(
                    "\x1b[30;42m{:>6}\x1b[0m root  20  0 {:>8}K {:>5.1}% \x1b[1m/usr/bin/proc-{row}\x1b[0m",
                    frame + row,
                    frame * 7 + row,
                    ((frame + row) % 100) as f64 / 3.0
                );
                out.push_str(&format!("\x1b[{row};1H{line:.width$}\x1b[K", width = columns));
            }
            out
        }
        other => unreachable!("{other}"),
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn run(columns: u16, rows: u16, workload: &str) {
    let mut cx = headless_context();
    let metrics = cx.update(|cx| {
        let text_system = gpui::WindowTextSystem::new(cx.text_system().clone());
        CellMetrics::measure(&font(), &text_system)
    });
    let width = metrics.cell_width * f32::from(columns) + px(1.);
    let height = metrics.line_height * f32::from(rows) + px(1.);
    let backend = FakeTerminalBackend::silent();
    let boxed = Box::new(backend.clone());
    let state = TerminalElementState::new();
    let handle = cx
        .open_window(size(width, height), |window, cx| {
            oxikube_runtime::init_deterministic(cx);
            let terminal =
                cx.new(|cx| TerminalState::new(boxed, TerminalSize::new(columns, rows), cx));
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let state = state.clone();
            cx.new(|_| Host {
                terminal,
                state,
                focus,
            })
        })
        .expect("headless window");
    let window: AnyWindowHandle = handle.into();

    let mut frames = Vec::with_capacity(FRAMES);
    let mut repaints = Vec::with_capacity(FRAMES);
    let mut allocations = Vec::with_capacity(FRAMES);
    let mut stats_before = state.cache_stats();
    let mut framed_misses = 0;
    for frame in 0..WARMUP + FRAMES {
        let bytes = wave(workload, frame, usize::from(rows), usize::from(columns));
        if !bytes.is_empty() {
            backend.output(bytes);
        }
        // The pump parses the wave; its coalesced notify waits for the frame tick.
        cx.run_until_parked();
        if frame == WARMUP {
            stats_before = state.cache_stats();
        }
        // The frame: the tick fires the notify and the window draws (prepaint shapes what changed).
        let misses_before = state.cache_stats().misses;
        let allocated = ALLOCATIONS.load(Ordering::Relaxed);
        let start = Instant::now();
        cx.advance_clock(FRAME_INTERVAL);
        let frame_ms = start.elapsed().as_secs_f64() * 1000.;
        let allocated = ALLOCATIONS.load(Ordering::Relaxed) - allocated;
        // A repaint with nothing new (hover, blink): every row from the cache.
        let start = Instant::now();
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
            .expect("draw");
        let repaint_ms = start.elapsed().as_secs_f64() * 1000.;
        if frame >= WARMUP {
            framed_misses += state.cache_stats().misses - misses_before;
            frames.push(frame_ms);
            repaints.push(repaint_ms);
            allocations.push(allocated);
        }
    }
    let stats = state.cache_stats();
    let hits = stats.hits - stats_before.hits;
    let misses = stats.misses - stats_before.misses;
    assert_eq!(
        framed_misses, misses,
        "every reshape happened in the timed frame"
    );
    let shaped = (stats.shaped_runs - stats_before.shaped_runs) as f64 / FRAMES as f64;
    frames.sort_by(f64::total_cmp);
    repaints.sort_by(f64::total_cmp);
    allocations.sort_unstable();
    println!(
        "| {columns}x{rows} | {workload} | {:.3} / {:.3} / {:.3} | {:.3} | {} | {:.1} % | {:.1} |",
        percentile(&frames, 0.5),
        percentile(&frames, 0.95),
        frames.last().copied().unwrap_or_default(),
        percentile(&repaints, 0.5),
        allocations[allocations.len() / 2],
        100. * hits as f64 / (hits + misses).max(1) as f64,
        shaped,
    );
}

fn main() {
    println!(
        "TerminalElement, headless ({} frames after {} warm-up, font {} 13 px)\n",
        FRAMES,
        WARMUP,
        TerminalFont::platform_family()
    );
    println!(
        "| grid | workload | frame p50 / p95 / max (ms) | repaint p50 (ms) | allocs/frame | row cache hits | runs shaped/frame |"
    );
    println!("|---|---|---|---|---|---|---|");
    for (columns, rows) in [(80, 24), (240, 60)] {
        for workload in ["idle", "yes", "ls", "htop"] {
            run(columns, rows, workload);
        }
    }
}
