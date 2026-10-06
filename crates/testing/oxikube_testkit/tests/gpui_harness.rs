//! The harness's own `#[gpui::test]`s (E05-S11): one example per helper. Later stories copy the
//! shape of these tests; `docs/testing-gpui.md` walks through them.
//!
//! Run: `cargo test -p oxikube_testkit --features gpui-test --test gpui_harness`.

mod support;

use std::time::Duration;

use gpui::{Entity, TestAppContext};
use oxikube_testkit::{TestPorts, gpui_test::TestApp};
use support::{Counter, Increment, Reset, SETTLE_AFTER, bindings};

/// Helper: open a window with a bound, focused counter.
fn counter_window(
    cx: &mut TestAppContext,
) -> (TestApp, oxikube_testkit::gpui_test::TestWindow<Counter>) {
    let mut app = TestApp::new(cx);
    app.bind_keys(bindings());
    let window = app.open_window(Counter::new);
    (app, window)
}

// `TestApp::open_window` + `TestWindow::read_root` / `update_root`.
#[gpui::test]
fn open_window_shows_the_root_view(cx: &mut TestAppContext) {
    let (_app, mut window) = counter_window(cx);
    assert_eq!(window.read_root(|counter, _| counter.count), 0);

    window.update_root(|counter, _, cx| {
        counter.count = 41;
        cx.notify();
    });
    assert_eq!(window.read_root(|counter, _| counter.count), 41);
    let root: Entity<Counter> = window.root();
    assert_eq!(window.read(|cx| root.read(cx).count), 41);
}

// `TestApp::bind_keys` + `TestWindow::simulate_keystrokes`: the key reaches the action through the
// keymap, like a user's key press.
#[gpui::test]
fn a_keystroke_runs_its_bound_action(cx: &mut TestAppContext) {
    let (_app, mut window) = counter_window(cx);
    window.simulate_keystrokes("j j j");
    assert_eq!(window.read_root(|counter, _| counter.count), 3);
    window.simulate_keystrokes("escape");
    assert_eq!(window.read_root(|counter, _| counter.count), 0);
}

#[gpui::test]
fn an_unbound_keystroke_does_nothing(cx: &mut TestAppContext) {
    let (_app, mut window) = counter_window(cx);
    window.simulate_keystrokes("k");
    assert_eq!(window.read_root(|counter, _| counter.count), 0);
}

// `TestWindow::dispatch_action`: what the command palette and the menus do.
#[gpui::test]
fn dispatch_action_reaches_the_focused_view(cx: &mut TestAppContext) {
    let (_app, mut window) = counter_window(cx);
    window.dispatch_action(Increment);
    window.dispatch_action(Increment);
    assert_eq!(window.read_root(|counter, _| counter.count), 2);
    window.dispatch_action(Reset);
    assert_eq!(window.read_root(|counter, _| counter.count), 0);
}

// `TestApp::advance_clock`: a debounce fires when the test clock reaches it, with no real waiting.
#[gpui::test]
fn advance_clock_fires_the_debounce(cx: &mut TestAppContext) {
    let (app, mut window) = counter_window(cx);
    window.simulate_keystrokes("j");
    assert!(!window.read_root(|counter, _| counter.settled));

    // Parked is not the same as elapsed: the timer is still pending.
    app.run_until_parked();
    assert!(!window.read_root(|counter, _| counter.settled));

    app.advance_clock(SETTLE_AFTER - Duration::from_millis(1));
    assert!(!window.read_root(|counter, _| counter.settled));
    app.advance_clock(Duration::from_millis(1));
    assert!(window.read_root(|counter, _| counter.settled));
}

// A new keystroke replaces the running debounce (the old task is dropped, which cancels it).
#[gpui::test]
fn a_second_keystroke_restarts_the_debounce(cx: &mut TestAppContext) {
    let (app, mut window) = counter_window(cx);
    window.simulate_keystrokes("j");
    window.advance_clock(Duration::from_millis(200));
    window.simulate_keystrokes("j");
    window.advance_clock(Duration::from_millis(200));
    // 400 ms after the first press, but only 200 after the second.
    assert!(!window.read_root(|counter, _| counter.settled));
    app.advance_clock(Duration::from_millis(100));
    assert!(window.read_root(|counter, _| counter.settled));
}

// `TestWindow::draw_frame` + `TestWindow::bounds`: layout is available after a frame.
#[gpui::test]
fn draw_frame_exposes_bounds_by_selector(cx: &mut TestAppContext) {
    let (_app, mut window) = counter_window(cx);
    window.draw_frame();
    let bounds = window
        .bounds("count")
        .expect("the count label was laid out");
    assert!(bounds.size.width > gpui::px(0.));
}

// `TestApp::update` runs against the `App` (globals, settings) and settles tasks.
#[gpui::test]
fn update_sets_and_reads_globals(cx: &mut TestAppContext) {
    struct Flag(bool);
    impl gpui::Global for Flag {}

    let mut app = TestApp::new(cx);
    app.update(|cx| cx.set_global(Flag(true)));
    assert!(app.read(|cx| cx.global::<Flag>().0));
}

// `TestPorts`: the fakes an `AppState` is built from, with handles kept for assertions.
#[gpui::test]
fn test_ports_are_seeded_and_clones_share_state(cx: &mut TestAppContext) {
    let _app = TestApp::new(cx);
    let ports = TestPorts::seeded();
    let again = ports.clone();
    assert_eq!(ports.resources.objects().len(), 5);
    again
        .resources
        .insert(oxikube_testkit::pod().name("extra").build());
    assert_eq!(ports.resources.objects().len(), 6);
}
