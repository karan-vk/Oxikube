//! A keypress must not allocate (E09-S06, docs/PERFORMANCE.md): the mapping hands out `&'static
//! str`s from tables for every common key, with or without modifiers.
//!
//! A counting global allocator wraps the system one for this test binary only.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use gpui::Keystroke;
use oxikube_terminal::grid::TerminalModes;
use oxikube_terminal::mappings::mouse::{MouseButton, MouseEvent, MouseKind};
use oxikube_terminal::mappings::{KeyMode, encode_mouse, to_esc_str, wheel_arrows};

struct Counting;

thread_local! {
    /// Per thread, so a test running beside this one does not count.
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn allocations() -> usize {
    ALLOCATIONS.with(Cell::get)
}

// SAFETY: forwards to the system allocator; only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|count| count.set(count.get() + 1));
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller upholds `GlobalAlloc::dealloc`'s contract.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn mapping_a_keypress_allocates_nothing() {
    let keystrokes: Vec<Keystroke> = [
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "pageup",
        "pagedown",
        "insert",
        "delete",
        "f1",
        "f4",
        "f5",
        "f12",
        "enter",
        "escape",
        "tab",
        "shift-tab",
        "backspace",
        "ctrl-a",
        "ctrl-c",
        "ctrl-z",
        "ctrl-space",
        "ctrl-[",
        "alt-b",
        "alt-f",
        "alt-.",
        "ctrl-alt-a",
        "shift-up",
        "ctrl-right",
        "ctrl-alt-shift-f9",
        "alt-enter",
        "alt-backspace",
        // Plain text maps to nothing, also without allocating.
        "a",
        "shift-a",
        "cmd-c",
    ]
    .iter()
    .map(|text| Keystroke::parse(text).expect("a keystroke"))
    .collect();
    let modes = [
        KeyMode::default(),
        KeyMode {
            app_cursor: true,
            option_as_meta: true,
        },
    ];

    let before = allocations();
    let mut produced = 0;
    for _ in 0..100 {
        for mode in modes {
            for keystroke in &keystrokes {
                produced += to_esc_str(keystroke, mode).map_or(0, |sequence| sequence.len());
            }
        }
    }
    let after = allocations();
    assert!(produced > 0);
    assert_eq!(after - before, 0, "to_esc_str allocated");
}

#[test]
fn encoding_a_mouse_report_allocates_nothing() {
    let modes = TerminalModes::MOUSE_DRAG | TerminalModes::SGR_MOUSE;
    let events = [
        MouseEvent {
            kind: MouseKind::Press(MouseButton::Left),
            column: 120,
            row: 40,
            modifiers: gpui::Modifiers::none(),
        },
        MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            column: 5,
            row: 7,
            modifiers: gpui::Modifiers::none(),
        },
    ];
    let before = allocations();
    let mut bytes = 0;
    for _ in 0..1000 {
        for event in &events {
            bytes += encode_mouse(event, modes).map_or(0, |report| report.as_bytes().len());
        }
        bytes += wheel_arrows(true, modes).len();
    }
    assert!(bytes > 0);
    assert_eq!(allocations() - before, 0);
}
