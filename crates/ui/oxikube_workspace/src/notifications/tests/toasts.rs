//! Toasts: a raised notification toasts with an "Open" button; a burst shows at most
//! `MAX_TOASTS` plus one summary; an update replaces its toast in place.

use gpui::{Entity, TestAppContext};
use oxikube_domain::command::Command;

use super::{Fixture, pod, prod, warning};
use crate::notifications::{MAX_TOASTS, SUMMARY_KEY, TOAST_WINDOW};
use crate::toast::{ToastLayer, ToastLevel};

fn layer(fx: &mut Fixture) -> Entity<ToastLayer> {
    let ws = fx.ws.clone();
    fx.vcx.update(|_, cx| ws.read(cx).toast_layer().clone())
}

/// The keys of the toasts visible or waiting.
fn toasts(fx: &mut Fixture) -> (Vec<String>, usize) {
    let layer = layer(fx);
    fx.vcx.update(|_, cx| {
        let layer = layer.read(cx);
        let keys = layer
            .visible()
            .iter()
            .map(|t| t.key.as_deref().unwrap_or("-").to_owned())
            .collect();
        (keys, layer.pending_len())
    })
}

#[gpui::test]
fn a_raised_warning_toasts_and_open_navigates(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    fx.submit(warning("web-0", "BackOff"));
    let layer = layer(&mut fx);
    let visible = fx.vcx.update(|_, cx| layer.read(cx).visible());
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].level, ToastLevel::Warning);
    assert_eq!(visible[0].message.as_ref(), "BackOff on web-0");
    let id = visible[0].id;

    fx.recorder.clear();
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    fx.click(&format!("toast-{}-action-0", id.as_u64()));
    let sent = fx.recorder.sent();
    assert_eq!(
        sent[1..],
        [
            Command::ClusterSelect { cluster: prod() },
            Command::ResourceOpen {
                target: pod("web-0")
            },
        ]
    );
    assert!(fx.service.list()[0].read);
}

#[gpui::test]
fn a_burst_shows_at_most_a_handful_of_toasts(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    for i in 0..1_000 {
        fx.service
            .submit(warning(&format!("pod-{i}"), &format!("Reason{}", i % 50)));
    }
    fx.settle();
    let (visible, pending) = toasts(&mut fx);
    assert!(
        visible.len() + pending <= MAX_TOASTS + 1,
        "{visible:?} + {pending}"
    );
    assert!(
        visible.iter().any(|key| key == SUMMARY_KEY) || pending > 0,
        "the rest are summed up: {visible:?}"
    );
    assert_eq!(fx.service.unread(), 500, "all listed, up to the bound");

    // The toasts go by themselves.
    fx.vcx.executor().advance_clock(TOAST_WINDOW * 3);
    fx.vcx.run_until_parked();
    assert_eq!(toasts(&mut fx), (Vec::new(), 0));
}

#[gpui::test]
fn the_toast_budget_comes_back_after_its_window(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    // The service's own limit is five per half minute; spread them out.
    for i in 0..MAX_TOASTS {
        fx.submit(warning(&format!("a{i}"), "BackOff"));
    }
    fx.submit(warning("over", "BackOff"));
    let (visible, pending) = toasts(&mut fx);
    assert_eq!(visible.len(), MAX_TOASTS, "{visible:?}");
    assert!(!visible.contains(&SUMMARY_KEY.to_owned()));
    assert_eq!(pending, 1, "the summary waits for a free slot");

    fx.vcx.executor().advance_clock(TOAST_WINDOW * 4);
    fx.vcx.run_until_parked();
    fx.submit(warning("later", "Failed"));
    let (visible, _) = toasts(&mut fx);
    assert_eq!(visible.len(), 1);
    assert_ne!(visible[0], SUMMARY_KEY);
}
