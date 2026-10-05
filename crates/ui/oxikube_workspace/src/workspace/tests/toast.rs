//! The toast layer: queue limit, deduplication by key, auto-dismiss on the test clock, actions,
//! focus, and that toasts do not lay out the window.

use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{Entity, Modifiers, VisualTestContext};

use super::*;
use crate::toast::{Toast, ToastAction, ToastId, ToastLayer};

fn layer(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Entity<ToastLayer> {
    vcx.update(|_, cx| ws.read(cx).toast_layer().clone())
}

fn show(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, toast: Toast) -> ToastId {
    let id = vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.show_toast(toast, cx)));
    vcx.run_until_parked();
    id
}

fn messages(layer: &Entity<ToastLayer>, vcx: &mut VisualTestContext) -> Vec<String> {
    vcx.update(|_, cx| {
        layer
            .read(cx)
            .visible()
            .iter()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

fn advance(vcx: &mut VisualTestContext, by: Duration) {
    vcx.executor().advance_clock(by);
    vcx.run_until_parked();
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[gpui::test]
fn the_queue_limits_visible_toasts_and_promotes_waiting_ones(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let max = vcx.update(|_, cx| layer.read(cx).max_visible());
    assert_eq!(max, 3);

    let ids: Vec<_> = (0..5)
        .map(|n| show(&ws, &mut vcx, Toast::info(format!("t{n}")).persistent()))
        .collect();
    assert_eq!(messages(&layer, &mut vcx), ["t0", "t1", "t2"]);
    assert_eq!(vcx.update(|_, cx| layer.read(cx).pending_len()), 2);
    assert!(
        bounds_named(&mut vcx, format!("toast-{}", ids[3].as_u64())).is_none(),
        "a waiting toast is not drawn"
    );
    assert!(bounds_named(&mut vcx, format!("toast-{}", ids[0].as_u64())).is_some());

    layer.update(&mut vcx, |layer, cx| layer.dismiss(ids[0], cx));
    vcx.run_until_parked();
    assert_eq!(messages(&layer, &mut vcx), ["t1", "t2", "t3"]);

    layer.update(&mut vcx, |layer, cx| layer.set_max_visible(1, cx));
    vcx.run_until_parked();
    assert_eq!(messages(&layer, &mut vcx), ["t1"]);
    assert_eq!(vcx.update(|_, cx| layer.read(cx).pending_len()), 3);
}

#[gpui::test]
fn a_toast_with_the_same_key_replaces_instead_of_stacking(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let first = show(
        &ws,
        &mut vcx,
        Toast::error("connect failed")
            .key("connect/prod")
            .persistent(),
    );
    let second = show(
        &ws,
        &mut vcx,
        Toast::error("connect failed again")
            .key("connect/prod")
            .persistent(),
    );
    show(&ws, &mut vcx, Toast::info("other").key("other"));
    assert_eq!(first, second);
    assert_eq!(
        messages(&layer, &mut vcx),
        ["connect failed again", "other"]
    );

    // A dismissed key can be shown again.
    layer.update(&mut vcx, |layer, cx| layer.dismiss_key("connect/prod", cx));
    vcx.run_until_parked();
    show(
        &ws,
        &mut vcx,
        Toast::error("third").key("connect/prod").persistent(),
    );
    assert_eq!(messages(&layer, &mut vcx), ["other", "third"]);
}

#[gpui::test]
fn toasts_auto_dismiss_when_the_clock_passes_their_timeout(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    show(&ws, &mut vcx, Toast::info("short").timeout(secs(2)));
    show(&ws, &mut vcx, Toast::success("default"));
    show(&ws, &mut vcx, Toast::error("sticky"));
    advance(&mut vcx, Duration::from_millis(1900));
    assert_eq!(messages(&layer, &mut vcx), ["short", "default", "sticky"]);
    advance(&mut vcx, Duration::from_millis(200));
    assert_eq!(messages(&layer, &mut vcx), ["default", "sticky"]);
    advance(&mut vcx, secs(2));
    assert_eq!(
        messages(&layer, &mut vcx),
        ["sticky"],
        "success defaults to 4 s"
    );
    advance(&mut vcx, secs(600));
    assert_eq!(
        messages(&layer, &mut vcx),
        ["sticky"],
        "an error stays until dismissed"
    );
}

#[gpui::test]
fn a_replaced_toast_restarts_its_timeout(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    show(&ws, &mut vcx, Toast::info("one").key("k").timeout(secs(4)));
    advance(&mut vcx, secs(3));
    show(&ws, &mut vcx, Toast::info("two").key("k").timeout(secs(4)));
    advance(&mut vcx, secs(3));
    assert_eq!(
        messages(&layer, &mut vcx),
        ["two"],
        "the first showing's timer did not fire"
    );
    advance(&mut vcx, secs(2));
    assert!(messages(&layer, &mut vcx).is_empty());
}

#[gpui::test]
fn a_promoted_toast_gets_its_full_timeout_from_when_it_appears(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    layer.update(&mut vcx, |layer, cx| layer.set_max_visible(1, cx));
    show(&ws, &mut vcx, Toast::info("first").timeout(secs(2)));
    show(&ws, &mut vcx, Toast::info("second").timeout(secs(2)));
    assert_eq!(messages(&layer, &mut vcx), ["first"]);
    advance(&mut vcx, Duration::from_millis(2100));
    assert_eq!(messages(&layer, &mut vcx), ["second"]);
    advance(&mut vcx, Duration::from_millis(1500));
    assert_eq!(
        messages(&layer, &mut vcx),
        ["second"],
        "it waited; its clock started on promotion"
    );
    advance(&mut vcx, secs(1));
    assert!(messages(&layer, &mut vcx).is_empty());
}

#[gpui::test]
fn an_action_button_runs_its_handler_and_dismisses(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let retried = Rc::new(Cell::new(0));
    let counter = retried.clone();
    let id = show(
        &ws,
        &mut vcx,
        Toast::error("connect failed").action(ToastAction::new("Retry", move |_, _| {
            counter.set(counter.get() + 1)
        })),
    );
    let button = bounds_named(&mut vcx, format!("toast-{}-action-0", id.as_u64()))
        .expect("the Retry button is drawn");
    vcx.simulate_click(center(button), Modifiers::none());
    assert_eq!(retried.get(), 1);
    assert!(messages(&layer, &mut vcx).is_empty());
}

#[gpui::test]
fn the_close_button_dismisses(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let id = show(&ws, &mut vcx, Toast::warning("careful").persistent());
    let button =
        bounds_named(&mut vcx, format!("toast-{}-close", id.as_u64())).expect("close button");
    vcx.simulate_click(center(button), Modifiers::none());
    assert!(messages(&layer, &mut vcx).is_empty());
}

#[gpui::test]
fn showing_a_toast_does_not_steal_focus_and_escape_gives_it_back(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let item = open(&ws, &mut vcx, "editor");
    assert!(item_focused(&ws, &mut vcx, item));

    show(&ws, &mut vcx, Toast::error("boom").persistent());
    assert!(
        item_focused(&ws, &mut vcx, item),
        "a new toast leaves focus alone"
    );

    vcx.update(|window, cx| {
        assert!(layer.update(cx, |layer, cx| layer.focus_toasts(window, cx)));
    });
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| layer.read(cx).is_focused(window, cx)));
    assert!(!item_focused(&ws, &mut vcx, item));

    vcx.simulate_keystrokes("escape");
    assert!(
        messages(&layer, &mut vcx).is_empty(),
        "Escape dismisses the focused toast"
    );
    assert!(
        item_focused(&ws, &mut vcx, item),
        "and focus goes back where it was"
    );
}

#[gpui::test]
fn focus_moves_to_the_next_toast_before_going_home(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let item = open(&ws, &mut vcx, "editor");
    show(&ws, &mut vcx, Toast::error("one").persistent());
    show(&ws, &mut vcx, Toast::error("two").persistent());
    vcx.update(|window, cx| {
        layer.update(cx, |layer, cx| layer.focus_toasts(window, cx));
    });
    vcx.run_until_parked();

    vcx.simulate_keystrokes("escape");
    assert_eq!(messages(&layer, &mut vcx), ["two"]);
    assert!(vcx.update(|window, cx| layer.read(cx).is_focused(window, cx)));
    vcx.simulate_keystrokes("escape");
    assert!(item_focused(&ws, &mut vcx, item));
}

#[gpui::test]
fn toasts_do_not_lay_out_the_window(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "editor");
    let before = bounds(&mut vcx, "item-editor").expect("item");
    let bar_before = bounds(&mut vcx, "status-bar").expect("bar");
    for n in 0..3 {
        show(&ws, &mut vcx, Toast::info(format!("t{n}")).persistent());
    }
    assert!(bounds(&mut vcx, "toast-layer").is_some());
    assert_eq!(bounds(&mut vcx, "item-editor"), Some(before));
    assert_eq!(bounds(&mut vcx, "status-bar"), Some(bar_before));

    // They sit in the bottom-right corner, above the status bar.
    let toast = bounds(&mut vcx, "toast-layer").expect("layer");
    assert!(toast.bottom() <= bar_before.origin.y);
    assert!(toast.right() <= bar_before.right());
    assert!(toast.origin.x > bar_before.origin.x + bar_before.size.width / 2.);
}

#[gpui::test]
fn a_toast_fades_in_within_the_cap_and_not_at_all_under_reduce_motion(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);

    // Reduce-motion off: the toast is drawn from the first frame (fading in) and the clock can
    // run past the fade without the test platform spinning on animation frames.
    vcx.update(|_, cx| crate::motion::set_reduce_motion(cx, false));
    show(&ws, &mut vcx, Toast::info("animated").timeout(secs(1)));
    assert!(
        vcx.update(|_, cx| crate::motion::animation_duration(cx, Duration::from_secs(5)))
            .is_some_and(|d| d <= crate::motion::MAX_ANIMATION)
    );
    assert!(bounds(&mut vcx, "toast-layer").is_some());
    advance(&mut vcx, Duration::from_millis(200));
    assert_eq!(messages(&layer, &mut vcx), ["animated"]);
    advance(&mut vcx, secs(1));
    assert!(messages(&layer, &mut vcx).is_empty());

    vcx.update(|_, cx| crate::motion::set_reduce_motion(cx, true));
    assert_eq!(
        vcx.update(|_, cx| crate::motion::animation_duration(cx, Duration::from_millis(100))),
        None
    );
    show(&ws, &mut vcx, Toast::info("still").timeout(secs(1)));
    assert!(bounds(&mut vcx, "toast-layer").is_some());
}
