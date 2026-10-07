//! The container selector and `pod::ViewLogs` through the controller.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_testkit::Timeline;

use super::fixture::{Fx, lines, pod_ref};
use crate::LogView;
use crate::view::OpenLogs;

#[gpui::test]
fn the_selector_lists_init_regular_and_ephemeral_containers(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(Timeline::immediate(lines(0, 3)).keep_open());
    let labels: Vec<String> = fx.read(&view, |v| {
        v.containers().iter().map(|c| c.label()).collect()
    });
    assert_eq!(
        labels,
        [
            "migrate (init)",
            "proxy (sidecar)",
            "app",
            "metrics",
            "debugger (ephemeral)"
        ]
    );
    assert_eq!(
        fx.read(&view, |v| v.options().container.clone()).as_deref(),
        Some("app"),
        "the annotated default"
    );
    assert!(fx.drawn("log-container"));
}

#[gpui::test]
fn selecting_a_container_reopens_the_stream_on_it(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(Timeline::immediate(lines(0, 3)).keep_open());
    fx.script(Timeline::immediate(lines(10, 2)).keep_open());
    fx.vcx.update(|_, cx| {
        view.update(cx, |view, cx| view.request_container("migrate", cx));
    });
    fx.settle();
    assert_eq!(
        fx.dispatcher.sent(),
        [Command::LogsSelectContainer {
            target: pod_ref(),
            container: "migrate".into()
        }]
    );
    let last = fx.opened().last().cloned().unwrap();
    assert_eq!(last.container.as_deref(), Some("migrate"));
    fx.read(&view, |v| {
        assert_eq!(v.options().container.as_deref(), Some("migrate"));
        assert_eq!(
            v.line_window().line_count(),
            2,
            "the new stream's lines only"
        );
        assert_eq!(v.row_text(0).as_deref(), Some("INFO line 10"));
    });
}

#[gpui::test]
fn view_logs_of_an_open_pod_shows_its_tab_again(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let first = fx.open(Timeline::immediate(lines(0, 3)).keep_open());
    fx.script(Timeline::immediate(lines(0, 1)).keep_open());
    let views = fx.views.clone();
    let again = fx
        .vcx
        .update(|window, cx| {
            views.update(cx, |views, cx| {
                let open = OpenLogs {
                    container: Some("metrics".into()),
                    ..OpenLogs::default()
                };
                views.open(&pod_ref(), &open, window, cx)
            })
        })
        .unwrap();
    fx.settle();
    assert_eq!(first, again, "one tab per pod");
    let open = fx
        .vcx
        .update(|_, cx| fx.workspace.read(cx).items_of_type::<LogView>().len());
    assert_eq!(open, 1);
    assert_eq!(
        fx.read(&first, |v| v.options().container.clone())
            .as_deref(),
        Some("metrics"),
        "switched to the asked container"
    );
}

#[gpui::test]
fn closing_the_tab_cancels_the_stream(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(Timeline::immediate(lines(0, 3)).keep_open());
    assert_eq!(fx.ports.logs.live_streams(), 1);
    let workspace = fx.workspace.clone();
    let id = view.entity_id();
    fx.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_item(id, window, cx);
        })
    });
    fx.settle();
    assert_eq!(
        fx.ports.logs.live_streams(),
        0,
        "the session went with the tab"
    );
}
