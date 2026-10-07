//! The rows: initial lines, virtualisation, the truncated marker and the state rows.

use gpui::TestAppContext;
use oxikube_app::logs::{EndReason, LogState};
use oxikube_domain::OxiError;
use oxikube_testkit::Timeline;
use oxikube_workspace::Item as _;

use super::fixture::{Fx, line, lines};
use crate::view::{OpenLogs, Row};

#[gpui::test]
fn the_initial_lines_render_with_the_tab_named_after_pod_and_container(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(Timeline::immediate(lines(0, 3)).keep_open());

    fx.read(&view, |view| {
        let window = view.line_window();
        assert_eq!(window.line_count(), 3);
        assert_eq!(window.state(), &LogState::Streaming);
        assert_eq!(view.row_text(0).as_deref(), Some("INFO line 0"));
        assert_eq!(view.row_text(2).as_deref(), Some("INFO line 2"));
    });
    // The pod was read: the default container is named, so the tab says which one.
    let title = fx.vcx.update(|_, cx| view.read(cx).tab_content(cx).title);
    assert_eq!(title.as_ref(), "web-0/app");
    assert!(fx.drawn("log-view"));
    assert!(fx.drawn("log-toolbar") || fx.drawn("log-range-tail"));
    // The stream asked the server for the tail with timestamps, following.
    let opened = fx.opened();
    assert_eq!(opened.len(), 1);
    assert!(opened[0].follow && opened[0].timestamps);
    assert_eq!(opened[0].tail_lines, Some(crate::view::TAIL_LINES));
    assert_eq!(
        opened[0].container.as_deref(),
        Some("app"),
        "a pod of several containers is read on its default one (the server needs a name)"
    );
}

#[gpui::test]
fn a_pod_that_cannot_be_read_still_streams_its_default_container(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.ports
        .resources
        .script()
        .get
        .push_err(OxiError::forbidden("pods \"web-0\" is forbidden"));
    let view = fx.open(Timeline::immediate(lines(0, 2)).keep_open());
    assert_eq!(fx.opened()[0].container, None);
    fx.read(&view, |view| {
        assert_eq!(view.line_window().line_count(), 2);
        assert!(view.containers().is_empty());
    });
}

#[gpui::test]
fn only_the_rows_on_screen_are_built(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(Timeline::immediate(lines(0, 5_000)).keep_open());
    fx.draw();
    let (count, built) = fx.read(&view, |v| (v.line_window().row_count(), v.rows_built()));
    assert_eq!(count, 5_000);
    assert!(built > 0, "something is drawn");
    assert!(
        built < 200,
        "{built} rows built for 5 000 lines: only the screen's (and a measuring row)"
    );
}

#[gpui::test]
fn dropped_lines_show_the_truncated_marker_on_top(cx: &mut TestAppContext) {
    let mut fx = Fx::with_buffer(cx, 100);
    let view = fx.open(Timeline::immediate(lines(0, 250)).keep_open());
    fx.read(&view, |view| {
        let window = view.line_window();
        assert_eq!(window.line_count(), 100);
        assert_eq!(window.row(0), Some(Row::Truncated(150)));
        assert_eq!(window.row(1), Some(Row::Line(150)));
        let marker = view.row_text(0).unwrap();
        assert!(marker.starts_with("150 older lines dropped"), "{marker}");
        assert!(marker.contains("100"), "names the bound: {marker}");
        assert_eq!(view.row_text(1).as_deref(), Some("INFO line 150"));
    });
}

#[gpui::test]
fn an_ended_stream_has_its_state_row_at_the_bottom(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    // The pod ran to its end: its stream ends, and the row says why (E08-S07).
    let mut done = super::fixture::pod().into_json();
    done["status"]["phase"] = serde_json::json!("Succeeded");
    fx.ports
        .resources
        .insert(oxikube_domain::Resource::from_json(done).unwrap());
    let view = fx.open(Timeline::immediate(lines(0, 2)));
    fx.read(&view, |view| {
        let window = view.line_window();
        assert_eq!(window.state(), &LogState::Ended(EndReason::PodFinished));
        assert_eq!(window.row(2), Some(Row::State));
        let text = view.row_text(2).unwrap();
        assert!(text.starts_with("Pod finished"), "{text}");
        assert_eq!(view.recovery(), None, "nothing to reconnect to");
    });
}

#[gpui::test]
fn a_failed_stream_says_why(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.ports
        .logs
        .script()
        .stream_logs
        .push_err(OxiError::forbidden(
            "pods \"web-0\" is forbidden: cannot get pods/log",
        ));
    let views = fx.views.clone();
    let view = fx
        .vcx
        .update(|window, cx| {
            views.update(cx, |views, cx| {
                views.open(&super::fixture::pod_ref(), &OpenLogs::default(), window, cx)
            })
        })
        .unwrap();
    fx.settle();
    fx.read(&view, |view| {
        assert!(matches!(view.line_window().state(), LogState::Failed(_)));
        let text = view.row_text(0).unwrap();
        assert!(
            text.starts_with("Not allowed to read these logs: pods \"web-0\" is forbidden"),
            "{text}"
        );
    });
}

#[gpui::test]
fn a_connecting_view_says_so_and_new_lines_append(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let clock = fx.ports.logs.clock().clone();
    let later = std::time::Duration::from_secs(1);
    let view = fx.open(
        Timeline::new()
            .ok_at(later, line(0))
            .ok_at(later * 2, line(1))
            .keep_open(),
    );
    fx.read(&view, |view| {
        assert_eq!(view.line_window().line_count(), 0);
    });
    clock.advance(later);
    fx.settle();
    clock.advance(later);
    fx.settle();
    fx.read(&view, |view| {
        assert_eq!(view.line_window().line_count(), 2);
        assert_eq!(view.row_text(1).as_deref(), Some("INFO line 1"));
    });
}
