//! Numbers for E09-S06 (docs/PERFORMANCE.md): what the input mapping layer costs per event.
//!
//! `cargo run --release -p oxikube_terminal --example input_bench`
//!
//! The mapping is a table lookup, so the budget "input to visible change <= 1 frame" is spent
//! entirely in the PTY round trip and the paint, not here. The end-to-end check (key -> echo ->
//! painted within one frame interval, deterministic clock) is the `#[gpui::test]`
//! `an_echo_is_painted_within_one_frame_of_the_key`; `tests/no_alloc.rs` proves the mapping does
//! not allocate.
#![allow(clippy::print_stdout)]

use std::hint::black_box;
use std::time::Instant;

use gpui::{Keystroke, Modifiers};
use oxikube_terminal::grid::TerminalModes;
use oxikube_terminal::mappings::mouse::{MouseButton, MouseEvent, MouseKind};
use oxikube_terminal::mappings::{KeyMode, encode_mouse, encode_paste, to_esc_str};

const ROUNDS: u32 = 2_000_000;

fn time(label: &str, mut f: impl FnMut()) {
    // Warm up, then take the best of five runs.
    for _ in 0..ROUNDS / 10 {
        f();
    }
    let best = (0..5)
        .map(|_| {
            let started = Instant::now();
            for _ in 0..ROUNDS {
                f();
            }
            started.elapsed()
        })
        .min()
        .unwrap_or_default();
    println!(
        "{label:<34} {:>7.1} ns/event",
        best.as_secs_f64() * 1e9 / f64::from(ROUNDS)
    );
}

fn main() {
    let parse = |text: &str| Keystroke::parse(text).expect("a keystroke");
    let (up, ctrl_c, f5, alt_b, text) = (
        parse("up"),
        parse("ctrl-c"),
        parse("ctrl-alt-shift-f5"),
        parse("alt-b"),
        parse("a"),
    );
    let mode = KeyMode {
        app_cursor: true,
        option_as_meta: true,
    };
    time("to_esc_str: arrow (DECCKM)", || {
        black_box(to_esc_str(black_box(&up), mode));
    });
    time("to_esc_str: ctrl-c", || {
        black_box(to_esc_str(black_box(&ctrl_c), mode));
    });
    time("to_esc_str: ctrl-alt-shift-f5", || {
        black_box(to_esc_str(black_box(&f5), mode));
    });
    time("to_esc_str: alt-b (meta)", || {
        black_box(to_esc_str(black_box(&alt_b), mode));
    });
    time("to_esc_str: plain text (None)", || {
        black_box(to_esc_str(black_box(&text), mode));
    });
    let click = MouseEvent {
        kind: MouseKind::Press(MouseButton::Left),
        column: 120,
        row: 40,
        modifiers: Modifiers::none(),
    };
    let sgr = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
    time("encode_mouse: SGR click", || {
        black_box(encode_mouse(black_box(&click), sgr));
    });
    let line = "kubectl get pods -A -o wide";
    time("encode_paste: one line, bracketed", || {
        black_box(encode_paste(black_box(line), true));
    });
}
