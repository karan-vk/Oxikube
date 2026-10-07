//! Non-gating throughput benchmark of the terminal grid and its bridge (E09-S04).
//!
//! `cargo run --release -p oxikube_terminal --example grid_bench`
//!
//! 1. Parse throughput of `TermGrid::advance` (80 x 24, 10 000 lines of scrollback) for three
//!    workloads: `yes` output, SGR-heavy coloured output (`ls --color`), and full-screen redraws
//!    with cursor addressing (`htop`), fed in 4 KiB chunks like a PTY read.
//! 2. `snapshot_into` cost at 80 x 24 and 240 x 70 (what the element pays per frame), and the
//!    cost of a resize with reflow over a full scrollback.
//! 3. The bridge under `yes`: a `FakeTerminalBackend` emits 4 KiB chunks at 50 MB/s of simulated
//!    time for 2 s through `TerminalState` (GPUI test clock, deterministic runtime): bytes parsed,
//!    notifies delivered to the observer per second (frame-coalesced: about 120/s, never one per
//!    chunk) and the wall time the pump spent.
#![allow(clippy::print_stdout)]

use std::cell::Cell;
use std::hint::black_box;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, TestAppContext};
use oxikube_ports::TerminalSize;
use oxikube_terminal::grid::{GridSearch, SEARCH_SLICE_LINES};
use oxikube_terminal::{TermGrid, TerminalScroll, TerminalSnapshot, TerminalState};
use oxikube_testkit::fakes::FakeTerminalBackend;

const CHUNK: usize = 4096;

fn chunks(pattern: &[u8], total: usize) -> Vec<Vec<u8>> {
    let stream: Vec<u8> = pattern.iter().copied().cycle().take(total).collect();
    stream.chunks(CHUNK).map(<[u8]>::to_vec).collect()
}

fn coloured_line(i: usize) -> String {
    format!(
        "\x1b[0m\x1b[01;34mdir-{i}\x1b[0m  \x1b[01;32mexec-{i}\x1b[0m  \x1b[38;5;208mfile-{i}.rs\x1b[0m\r\n"
    )
}

fn htop_frame(frame: usize) -> String {
    let mut out = String::from("\x1b[H");
    for row in 1..=24 {
        out.push_str(&format!(
            "\x1b[{row};1H\x1b[30;42m{:>5}\x1b[0m root  20  0 {:>7}K {:>5.1}% \x1b[1m/usr/bin/proc-{row}\x1b[K",
            frame + row,
            frame * 7 + row,
            (frame % 100) as f64 / 3.0
        ));
    }
    out
}

fn parse(name: &str, data: &[Vec<u8>]) {
    let mut grid = TermGrid::new(TerminalSize::new(80, 24), 10_000);
    let mut events = Vec::new();
    let bytes: usize = data.iter().map(Vec::len).sum();
    let start = Instant::now();
    for chunk in data {
        grid.advance(chunk, &mut events);
        events.clear();
    }
    let elapsed = start.elapsed();
    println!(
        "parse {name:<28} {:>8.1} MB/s  ({} MB in {:.0} ms)",
        bytes as f64 / elapsed.as_secs_f64() / 1e6,
        bytes / 1_000_000,
        elapsed.as_secs_f64() * 1e3
    );
}

fn snapshot_cost(columns: u16, rows: u16) {
    const N: u32 = 2_000;
    let mut grid = TermGrid::new(TerminalSize::new(columns, rows), 10_000);
    let mut events = Vec::new();
    for i in 0..2_000 {
        grid.advance(coloured_line(i).as_bytes(), &mut events);
    }
    let mut snapshot = TerminalSnapshot::default();
    grid.snapshot_into(&mut snapshot);
    let start = Instant::now();
    for _ in 0..N {
        grid.snapshot_into(black_box(&mut snapshot));
    }
    println!(
        "snapshot_into {columns}x{rows:<20} {:>8.1} us/frame",
        start.elapsed().as_secs_f64() * 1e6 / f64::from(N)
    );
}

fn resize_cost() {
    let mut grid = TermGrid::new(TerminalSize::new(120, 40), 10_000);
    let mut events = Vec::new();
    for i in 0..12_000 {
        grid.advance(coloured_line(i).as_bytes(), &mut events);
    }
    grid.scroll(TerminalScroll::Bottom);
    let start = Instant::now();
    let mut steps = 0u32;
    for width in (80..120).chain((80..120).rev()) {
        grid.resize(TerminalSize::new(width, 40));
        steps += 1;
    }
    println!(
        "resize 10k-line scrollback           {:>8.1} ms/step (reflow, {steps} steps)",
        start.elapsed().as_secs_f64() * 1e3 / f64::from(steps)
    );
}

fn search_cost() {
    let mut grid = TermGrid::new(TerminalSize::new(120, 40), 10_000);
    let mut events = Vec::new();
    for i in 0..12_000 {
        grid.advance(coloured_line(i).as_bytes(), &mut events);
    }
    for pattern in ["exec-11999", "file-\\d+7\\.rs"] {
        let start = Instant::now();
        let found = grid.search(pattern).expect("valid pattern").len();
        println!(
            "search 10k lines {pattern:<19} {:>8.1} ms ({found} matches)",
            start.elapsed().as_secs_f64() * 1e3
        );
        // What the UI thread can wait behind: the longest single slice (one hold of the lock).
        let mut search = GridSearch::new(pattern).expect("valid pattern");
        let mut longest = Duration::ZERO;
        loop {
            let start = Instant::now();
            let done = search.step(&grid);
            longest = longest.max(start.elapsed());
            if done {
                break;
            }
        }
        println!(
            "search slice ({SEARCH_SLICE_LINES} lines) {pattern:<13} {:>8.3} ms (longest hold)",
            longest.as_secs_f64() * 1e3
        );
    }
}

fn bridge(cx: &mut TestAppContext) {
    const RATE_BYTES_PER_SEC: usize = 50_000_000;
    const SECONDS: u32 = 2;
    const STEP: Duration = Duration::from_millis(1);
    cx.update(oxikube_runtime::init_deterministic);
    let backend = FakeTerminalBackend::silent();
    let terminal =
        cx.new(|cx| TerminalState::new(Box::new(backend.clone()), TerminalSize::new(80, 24), cx));
    let notifies = Rc::new(Cell::new(0u64));
    let counter = notifies.clone();
    cx.update(|cx| {
        cx.observe(&terminal, move |_, _| counter.set(counter.get() + 1))
            .detach()
    });

    let yes = b"y\r\n".repeat(CHUNK / 3);
    let per_step = RATE_BYTES_PER_SEC / 1_000 / CHUNK;
    let steps = SECONDS * 1_000;
    let mut sent = 0usize;
    let start = Instant::now();
    for _ in 0..steps {
        for _ in 0..per_step {
            backend.output(yes.clone());
            sent += yes.len();
        }
        cx.executor().advance_clock(STEP);
        cx.run_until_parked();
    }
    let wall = start.elapsed();
    println!(
        "bridge yes @ 50 MB/s for {SECONDS} s           {:>8.0} notifies/s ({} chunks/s), {:.1} MB/s parsed wall",
        notifies.get() as f64 / f64::from(SECONDS),
        sent / yes.len() / SECONDS as usize,
        sent as f64 / wall.as_secs_f64() / 1e6
    );
    let row = terminal.read_with(cx, |terminal, _| terminal.snapshot().row_text(0));
    assert_eq!(row, "y");
}

fn main() {
    // A PTY turns `\n` into `\r\n` (ONLCR); the fake backend does not, so feed what a PTY sends.
    parse("yes", &chunks(b"y\r\n", 64 << 20));
    let coloured: String = (0..200_000).map(coloured_line).collect();
    parse(
        "ls --color (SGR heavy)",
        &chunks(coloured.as_bytes(), coloured.len()),
    );
    let htop: String = (0..3_000).map(htop_frame).collect();
    parse(
        "htop (cursor addressing)",
        &chunks(htop.as_bytes(), htop.len()),
    );
    snapshot_cost(80, 24);
    snapshot_cost(240, 70);
    resize_cost();
    search_cost();
    bridge(&mut TestAppContext::single());
}
