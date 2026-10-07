//! The toolbar (E08-U556): what is on the primary row, what is in the "..." menu, that nothing
//! clips at 1024 px or at UI scale 1.5, the range dropdown, the container picker and the hint of
//! a crash-looping container.

use gpui::{Bounds, Pixels, TestAppContext, px, size};
use oxikube_domain::command::Command;
use oxikube_domain::log::LogRange;
use oxikube_testkit::Timeline;
use oxikube_ui::{UiScale, set_ui_scale};
use serde_json::json;

use super::fixture::{Fx, lines, pod_ref};
use crate::LogView;

/// Everything on the primary row, left to right (the breadcrumb is not a control).
const PRIMARY: [&str; 7] = [
    "log-container",
    "log-range",
    "log-find",
    "log-previous",
    "log-wrap",
    "log-autoscroll",
    "log-overflow",
];

/// What the menu holds (kubectl installed, plain-text log).
const OVERFLOW: [&str; 8] = [
    "log-timestamps",
    "log-mark",
    "log-copy",
    "log-send-to-agent",
    "log-save",
    "log-clear",
    "log-tail-in-terminal",
    "log-fullscreen",
];

fn open(fx: &mut Fx) -> gpui::Entity<LogView> {
    for _ in 0..4 {
        fx.script(Timeline::immediate(lines(0, 3)).keep_open());
    }
    fx.open(Timeline::immediate(lines(0, 30)).keep_open())
}

fn bounds(fx: &mut Fx, selector: &'static str) -> Bounds<Pixels> {
    fx.vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
}

#[gpui::test]
fn the_primary_row_holds_the_few_controls_and_the_rest_is_in_the_menu(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    fx.draw();
    for selector in PRIMARY {
        assert!(fx.drawn(selector), "{selector} is on the primary row");
    }
    for selector in OVERFLOW {
        assert!(!fx.drawn(selector), "{selector} is only in the menu");
    }
    assert!(!fx.drawn("log-json"));
    assert_eq!(fx.overflow_ids(&view), OVERFLOW, "in this order");
    assert!(fx.drawn("log-toolbar"));
}

#[gpui::test]
fn every_menu_entry_sends_its_command(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    let target = pod_ref;
    fx.overflow("log-timestamps");
    fx.overflow("log-copy");
    fx.overflow("log-send-to-agent");
    fx.overflow("log-mark");
    fx.overflow("log-fullscreen");
    fx.overflow("log-clear");
    let sent = fx.dispatcher.sent();
    assert_eq!(
        sent[..6],
        [
            Command::LogsToggleTimestamps { target: target() },
            Command::LogsCopy { target: target() },
            Command::LogsSendToAgent { target: target() },
            Command::LogsMark { target: target() },
            Command::LogsToggleFullscreen { target: target() },
            Command::LogsClear { target: target() },
        ]
    );
    assert!(fx.read(&view, |v| v.options().timestamps));
}

#[gpui::test]
fn save_and_the_range_rows_are_commands_too(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    fx.click("log-range");
    fx.click("log-range-15m");
    assert_eq!(fx.read(&view, |v| v.options().range), LogRange::Last15m);
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsSetRange {
            target: pod_ref(),
            range: LogRange::Last15m
        })
    );
    fx.overflow("log-save");
    assert!(matches!(
        fx.dispatcher.sent().last(),
        Some(Command::LogsSave { .. })
    ));
}

/// Every primary control and the breadcrumb lies inside the toolbar, which lies inside the view.
fn assert_nothing_clips(fx: &mut Fx, case: &str) {
    fx.draw();
    let view = bounds(fx, "log-view");
    let toolbar = bounds(fx, "log-toolbar");
    assert!(
        toolbar.right() <= view.right() + px(0.5),
        "{case}: the toolbar is wider than the view"
    );
    for selector in PRIMARY {
        let control = bounds(fx, selector);
        assert!(
            control.left() >= toolbar.left() - px(0.5)
                && control.right() <= toolbar.right() + px(0.5),
            "{case}: {selector} clips: {control:?} in {toolbar:?}"
        );
        assert!(
            control.top() >= toolbar.top() && control.bottom() <= toolbar.bottom() + px(0.5),
            "{case}: {selector} is taller than the toolbar"
        );
    }
    let title = bounds(fx, "log-title");
    assert!(title.right() <= bounds(fx, "log-container").left());
    // The first row sits below the toolbar, not under it.
    let body = bounds(fx, "log-body");
    assert!(body.top() >= toolbar.bottom() - px(0.5), "{case}");
}

#[gpui::test]
fn nothing_clips_at_1024_px_or_at_ui_scale_1_5(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open(&mut fx);
    for (width, scale) in [
        (1280., 1.0),
        (1024., 1.0),
        (1024., 1.5),
        (1280., 1.5),
        (790., 1.5),
        (790., 1.0),
    ] {
        fx.vcx.simulate_resize(size(px(width), px(700.)));
        fx.vcx.update(|_, cx| set_ui_scale(cx, UiScale::new(scale)));
        fx.settle();
        assert_nothing_clips(&mut fx, &format!("{width} px at {scale}"));
    }
    fx.vcx.update(|_, cx| set_ui_scale(cx, UiScale::IDENTITY));
}

#[gpui::test]
fn the_title_gives_way_first(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open(&mut fx);
    fx.vcx.simulate_resize(size(px(1280.), px(700.)));
    fx.settle();
    let wide = bounds(&mut fx, "log-title").size.width;
    fx.vcx.simulate_resize(size(px(240.), px(700.)));
    fx.settle();
    let narrow = bounds(&mut fx, "log-title").size.width;
    assert!(narrow < wide, "{narrow:?} < {wide:?}");
}

#[gpui::test]
fn the_picker_names_the_container_and_counts_them(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    // Five containers: init, sidecar, app (the annotated default), metrics, ephemeral.
    let label = fx.read(&view, |v| v.container_picker_label());
    assert_eq!(label, "app (3/5)");
    fx.click("log-container");
    fx.click("log-container:metrics");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsSelectContainer {
            target: pod_ref(),
            container: "metrics".into()
        })
    );
    assert_eq!(
        fx.read(&view, |v| v.container_picker_label()),
        "metrics (4/5)"
    );
}

#[gpui::test]
fn a_range_button_names_the_range_and_the_tail_length(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    assert_eq!(fx.read(&view, |v| v.range_button_label()), "tail 1000");
    fx.keys("3");
    assert_eq!(fx.read(&view, |v| v.range_button_label()), "since 5m");
    fx.keys("1");
    assert_eq!(fx.read(&view, |v| v.range_button_label()), "head");
}

fn crash_looping_pod() -> oxikube_domain::Resource {
    oxikube_domain::Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": "web-0", "namespace": "shop"},
        "spec": {"containers": [{"name": "app"}, {"name": "metrics"}]},
        "status": {"phase": "Running", "containerStatuses": [
            {"name": "app", "restartCount": 7,
             "state": {"waiting": {"reason": "CrashLoopBackOff"}},
             "lastState": {"terminated": {"exitCode": 1, "reason": "Error"}}},
            {"name": "metrics", "restartCount": 0, "state": {"running": {}}}
        ]}
    }))
    .expect("a pod")
}

#[gpui::test]
fn a_crash_looping_container_hints_that_previous_holds_the_last_crash(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.ports.resources.insert(crash_looping_pod());
    let view = open(&mut fx);
    assert!(fx.read(&view, LogView::shows_crash_hint));
    fx.draw();
    assert!(fx.drawn("log-crash-hint"));

    // The hint's button is the Previous command; with the previous instance shown it goes away.
    fx.click("log-crash-previous");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsTogglePrevious { target: pod_ref() })
    );
    assert!(fx.read(&view, |v| v.options().previous));
    fx.draw();
    assert!(!fx.drawn("log-crash-hint"));

    // A healthy container of the same pod has no hint.
    fx.click("log-container");
    fx.click("log-container:metrics");
    fx.draw();
    assert!(!fx.drawn("log-crash-hint"));
}

#[gpui::test]
fn a_healthy_pod_has_no_crash_hint(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    assert!(!fx.read(&view, LogView::shows_crash_hint));
    fx.draw();
    assert!(!fx.drawn("log-crash-hint"));
}

#[gpui::test]
fn the_oldest_row_on_screen_is_whole_under_the_toolbar(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(Timeline::immediate(lines(0, 200)).keep_open());
    let mut slacks = Vec::new();
    for (height, scale) in [(700., 1.0), (713., 1.0), (713., 1.5), (655., 1.25)] {
        fx.vcx.simulate_resize(size(px(1024.), px(height)));
        fx.vcx.update(|_, cx| set_ui_scale(cx, UiScale::new(scale)));
        fx.settle();
        fx.settle();
        fx.draw();
        let body = bounds(&mut fx, "log-body");
        let rows = bounds(&mut fx, "log-rows");
        let row = fx.read(&view, |v| v.row_height());
        let whole = rows.size.height / row;
        let ctx = format!("{height} px at {scale}: body {body:?}, rows {rows:?}, row {row:?}");
        // The list is a whole number of rows tall and sits flush with the bottom of the body, so
        // the extra space is above it and the oldest row on screen is never cut by the toolbar.
        assert!(
            (whole - whole.round()).abs() < 0.02,
            "list is {whole} rows tall: {ctx}"
        );
        assert!(
            (rows.bottom() - body.bottom()).abs() < px(0.5),
            "the list ends at the body's bottom: {ctx}"
        );
        let gap = rows.top() - body.top();
        assert!(
            gap >= px(0.) && gap < row,
            "less than a row of slack above the list: {ctx}"
        );
        slacks.push(gap);
    }
    assert!(
        slacks.iter().any(|gap| *gap > px(1.)),
        "the sizes cover a body that is not a whole number of rows: {slacks:?}"
    );
    fx.vcx.update(|_, cx| set_ui_scale(cx, UiScale::IDENTITY));
}
