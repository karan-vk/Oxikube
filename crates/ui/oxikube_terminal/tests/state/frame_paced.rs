//! A `yes`-like flood into a terminal while a window presents frames, some of them late: the
//! terminal is notified at most once between two frames (E05-P599, the `terminal` scenario's
//! `yes-flood`, budget `max_view_notifies_per_frame` ≤ 1). With the old 8.33 ms coalescing timer a
//! frame one refresh late found two notifies. A frame is simulated the way the platform delivers
//! one (`Window::simulate_next_frame` runs the next-frame callbacks); the test platform draws a
//! dirty window as soon as an update flushes, so the count is of notifies, not of renders.

use gpui::{
    AnyWindowHandle, AppContext as _, Context, IntoElement, Render, TestAppContext, Window,
};
use oxikube_runtime::FRAME_INTERVAL;
use oxikube_testkit::fakes::FakeTerminalBackend;

use super::harness;

/// The window's content; the window is what presents frames.
struct Screen;

impl Render for Screen {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
    }
}

fn open_window(cx: &mut TestAppContext) -> AnyWindowHandle {
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| cx.new(|_| Screen))
            .unwrap()
            .into()
    });
    cx.run_until_parked();
    window
}

fn frame(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn a_flood_with_late_frames_notifies_at_most_once_per_frame(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (80, 24));
    let window = open_window(cx);
    frame(window, cx);

    let step = FRAME_INTERVAL / 8;
    let mut per_frame = Vec::new();
    for refresh in 0..240u32 {
        let before = h.notifies.get();
        // Output at 8 points per refresh, as a PTY reader delivers a flood in chunks.
        for _ in 0..8 {
            for _ in 0..16 {
                backend.output("y\r\n");
            }
            cx.executor().advance_clock(step);
            cx.run_until_parked();
        }
        // Every fifth frame is one to three refreshes late; output keeps arriving meanwhile.
        if refresh % 5 == 4 {
            for _ in 0..=refresh % 3 {
                backend.output("y\r\n");
                cx.executor().advance_clock(FRAME_INTERVAL);
                cx.run_until_parked();
            }
        }
        frame(window, cx);
        per_frame.push(h.notifies.get() - before);
    }
    backend.output("\r\nflood-done");
    cx.run_until_parked();
    frame(window, cx);

    let notified = per_frame.iter().filter(|&&n| n > 0).count();
    assert!(
        notified > 200,
        "the flood is drawn frame after frame: {notified} of 240 frames"
    );
    let worst = per_frame.iter().copied().max().unwrap_or(0);
    assert_eq!(worst, 1, "at most one notify of the terminal per frame");
    assert_eq!(h.row(cx, 23), "flood-done", "every chunk reached the grid");
}
