//! The multi-pod view: a Deployment's pods merged by server timestamp, one colour and gutter per
//! pod, the banner, switching sources off, the stream cap, and the stream lifecycle.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_app::logs::{LogConfig, LogState};
use oxikube_domain::command::Command;
use oxikube_testkit::{ScriptedFeed, TICK, Timeline};
use oxikube_workspace::Item as _;

use super::fixture::{Fx, deployment_ref, pod_line, web_pod};
use crate::view::{BANNER_SECONDS, colour_index};

fn timeline(
    lines: impl IntoIterator<Item = oxikube_domain::log::LogLine>,
) -> Timeline<oxikube_domain::log::LogLine> {
    Timeline::immediate(lines).keep_open()
}

/// Rows of the view, as drawn (gutter, then text).
fn rows(fx: &mut Fx, view: &gpui::Entity<crate::LogView>) -> Vec<String> {
    fx.read(view, |view| {
        (0..view.line_window().row_count())
            .filter_map(|i| view.row_text(i))
            .collect()
    })
}

#[gpui::test]
fn a_deployments_pods_are_merged_by_timestamp_with_a_gutter_per_pod(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    fx.seed_web(vec![
        (
            "web-7d9-aaaaa",
            timeline([pod_line("web-7d9-aaaaa", 0), pod_line("web-7d9-aaaaa", 3)]),
        ),
        (
            "web-7d9-bbbbb",
            timeline([pod_line("web-7d9-bbbbb", 1), pod_line("web-7d9-bbbbb", 2)]),
        ),
    ]);
    let view = fx.open_web();

    assert_eq!(
        rows(&mut fx, &view),
        [
            "aaaaa INFO web-7d9-aaaaa line 0",
            "bbbbb INFO web-7d9-bbbbb line 1",
            "bbbbb INFO web-7d9-bbbbb line 2",
            "aaaaa INFO web-7d9-aaaaa line 3",
        ],
        "merged by server timestamp, each line led by its pod's short name"
    );
    fx.read(&view, |view| {
        assert!(view.is_aggregate());
        assert_eq!(view.line_window().state(), &LogState::Streaming);
        let a = view.prefix_of("web-7d9-aaaaa", "app").expect("a gutter");
        let b = view.prefix_of("web-7d9-bbbbb", "app").expect("a gutter");
        assert_eq!(a.text.len(), b.text.len(), "one fixed width");
        assert_eq!(
            a.colour,
            colour_index("web-7d9-aaaaa", 10),
            "hash of the pod name"
        );
        assert_eq!(b.colour, colour_index("web-7d9-bbbbb", 10));
    });
    let title = fx.vcx.update(|_, cx| view.read(cx).tab_content(cx).title);
    assert_eq!(title.as_ref(), "deployment/web");
    assert!(
        fx.drawn("log-sources"),
        "the toolbar has the Sources menu, not a container selector"
    );
    assert!(!fx.drawn("log-container"));
    let streams = fx.opened();
    assert_eq!(streams.len(), 2, "one stream per pod");
    assert!(streams.iter().all(|o| o.timestamps && o.follow));
}

#[gpui::test]
fn workload_view_logs_on_the_bus_opens_the_merged_view_and_a_pod_command_refuses_it(
    cx: &mut TestAppContext,
) {
    use oxikube_workspace::CommandDispatcher as _;
    let mut fx = Fx::merged(cx);
    fx.ports.resources.insert(super::fixture::deployment());
    let tiered = oxikube_testkit::pod()
        .name("web-a")
        .namespace("shop")
        .label("app", "web")
        .label("tier", "api")
        .build();
    fx.ports.resources.insert(tiered);
    fx.script(timeline([pod_line("web-a", 0)]));
    let command = Command::WorkloadViewLogs {
        target: deployment_ref(),
        selector: Some("tier=api".into()),
        container: Some("app".into()),
        follow: true,
        tail_lines: Some(50),
    };
    fx.vcx
        .update(|_, cx| fx.dispatcher.dispatch(command.clone(), cx));
    fx.settle();
    fx.pass(Duration::from_millis(800));
    let views = fx
        .vcx
        .update(|_, cx| fx.workspace.read(cx).items_of_type::<crate::LogView>());
    assert_eq!(views.len(), 1, "one merged tab");
    assert!(fx.read(&views[0], |v| v.is_aggregate()));
    let options = fx.read(&views[0], |v| v.options().clone());
    assert_eq!(options.selector.as_deref(), Some("tier=api"));
    assert_eq!(options.container.as_deref(), Some("app"));
    // The selector narrows the watch, ANDed with the Deployment's own.
    let watches: Vec<_> = fx
        .ports
        .resources
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            oxikube_testkit::ResourceCall::Watch { options, .. } => options.label_selector,
            _ => None,
        })
        .collect();
    assert_eq!(watches, ["app=web,tier=api"]);
    let streams = fx.opened();
    assert_eq!(streams[0].tail_lines, Some(50));

    // Opening it again shows that tab instead of a second one.
    fx.vcx
        .update(|_, cx| fx.dispatcher.dispatch(command.clone(), cx));
    fx.settle();
    let again = fx
        .vcx
        .update(|_, cx| fx.workspace.read(cx).items_of_type::<crate::LogView>());
    assert_eq!(again.len(), 1);
}

#[gpui::test]
fn a_pod_that_joins_or_leaves_shows_a_banner_that_clears_itself(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    fx.ports.resources.insert(super::fixture::deployment());
    fx.ports.resources.insert(web_pod("web-a"));
    ScriptedFeed::new()
        .initial([web_pod("web-a")])
        .add(1, web_pod("web-b"))
        .delete(2, web_pod("web-a"))
        .install(&fx.ports.resources);
    fx.script(timeline([pod_line("web-a", 0)]));
    fx.script(timeline([pod_line("web-b", 1)]));
    let view = fx.open_web();
    assert!(
        fx.read(&view, |v| v.banner_lines().is_empty()),
        "the first list is no news"
    );
    assert!(!fx.drawn("log-banner"));

    fx.ports.resources.clock().advance(TICK);
    fx.pass(Duration::from_millis(100));
    assert_eq!(fx.read(&view, |v| v.banner_lines()), ["pod web-b added"]);
    assert!(fx.drawn("log-banner"));
    assert_eq!(
        fx.read(&view, |v| v.sources().len()),
        2,
        "the new pod is streamed"
    );

    fx.ports.resources.clock().advance(TICK);
    fx.pass(Duration::from_millis(100));
    assert_eq!(
        fx.read(&view, |v| v.banner_lines()),
        ["pod web-b added", "pod web-a ended"]
    );

    // It goes away by itself after a while.
    fx.vcx
        .executor()
        .advance_clock(Duration::from_secs(BANNER_SECONDS + 1));
    fx.vcx.run_until_parked();
    assert!(fx.read(&view, |v| v.banner_lines().is_empty()));
}

#[gpui::test]
fn the_banner_can_be_dismissed(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    fx.ports.resources.insert(super::fixture::deployment());
    ScriptedFeed::new()
        .initial([web_pod("web-a")])
        .add(1, web_pod("web-b"))
        .install(&fx.ports.resources);
    fx.script(timeline([]));
    fx.script(timeline([]));
    let view = fx.open_web();
    fx.ports.resources.clock().advance(TICK);
    fx.pass(Duration::from_millis(100));
    assert_eq!(fx.read(&view, |v| v.banner_lines().len()), 1);
    fx.click("log-banner-dismiss");
    assert!(fx.read(&view, |v| v.banner_lines().is_empty()));
    assert!(!fx.drawn("log-banner"));
}

#[gpui::test]
fn a_source_switched_off_leaves_the_rows_and_comes_back_while_its_stream_keeps_running(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::merged(cx);
    fx.seed_web(vec![
        (
            "web-a",
            Timeline::new()
                .ok_at(Duration::ZERO, pod_line("web-a", 0))
                .ok_at(Duration::from_secs(5), pod_line("web-a", 4))
                .keep_open(),
        ),
        (
            "web-b",
            Timeline::new()
                .ok_at(Duration::ZERO, pod_line("web-b", 1))
                .ok_at(Duration::from_secs(5), pod_line("web-b", 5))
                .keep_open(),
        ),
    ]);
    let view = fx.open_web();
    assert_eq!(rows(&mut fx, &view).len(), 2);

    fx.vcx.update(|_, cx| {
        view.update(cx, |view, cx| view.request_toggle_source("web-a", None, cx));
    });
    fx.pass(Duration::from_millis(100));
    fx.read(&view, |view| {
        assert!(view.is_source_hidden("web-a", "app"));
        assert!(view.line_window().is_level_filtered());
    });
    assert_eq!(rows(&mut fx, &view), ["b INFO web-b line 1"]);

    // The hidden pod keeps streaming: its new line is in the buffer, not in the rows.
    fx.pass(Duration::from_secs(6));
    assert_eq!(
        rows(&mut fx, &view),
        ["b INFO web-b line 1", "b INFO web-b line 5"]
    );
    assert_eq!(fx.ports.logs.live_streams(), 2, "hidden is not stopped");
    let buffered = fx.read(&view, |v| v.session().expect("a session").len());
    assert_eq!(buffered, 4);

    // Switching it back on shows everything it said meanwhile, in place.
    fx.vcx.update(|_, cx| {
        view.update(cx, |view, cx| view.request_toggle_source("web-a", None, cx));
    });
    fx.pass(Duration::from_millis(100));
    assert_eq!(
        rows(&mut fx, &view),
        [
            "a INFO web-a line 0",
            "b INFO web-b line 1",
            "a INFO web-a line 4",
            "b INFO web-b line 5",
        ]
    );
    fx.read(&view, |view| {
        assert!(!view.line_window().is_level_filtered())
    });
    let sent = fx.dispatcher.sent();
    assert!(
        sent.iter()
            .any(|c| matches!(c, Command::LogsToggleSource { pod, .. } if pod == "web-a")),
        "the toggle went through the bus: {sent:?}"
    );
}

#[gpui::test]
fn switching_sources_in_a_wrapped_view_keeps_the_rows_consistent(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    fx.seed_web(vec![
        ("web-a", timeline((0..40).map(|i| pod_line("web-a", i * 2)))),
        (
            "web-b",
            timeline((0..40).map(|i| pod_line("web-b", i * 2 + 1))),
        ),
    ]);
    let view = fx.open_web();
    fx.keys("w");
    fx.vcx.update(|_, cx| {
        view.update(cx, |view, cx| view.request_toggle_source("web-b", None, cx));
    });
    fx.pass(Duration::from_millis(100));
    let (count, filtered) = fx.read(&view, |v| {
        (
            v.line_window().line_count(),
            v.line_window().is_level_filtered(),
        )
    });
    assert_eq!((count, filtered), (40, true));
    fx.draw();
    assert!(fx.read(&view, |v| v.rows_built()) > 0, "wrapped rows drew");
    fx.vcx.update(|_, cx| {
        view.update(cx, |view, cx| view.request_toggle_source("web-b", None, cx));
    });
    fx.pass(Duration::from_millis(100));
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 80);
}

#[gpui::test]
fn pods_over_the_stream_cap_are_counted_in_a_notice_and_a_raised_cap_starts_them(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::with_config(
        cx,
        LogConfig {
            max_streams: 1,
            ..LogConfig::default()
        },
    );
    fx.seed_web(vec![
        ("web-a", timeline([pod_line("web-a", 0)])),
        ("web-b", timeline([pod_line("web-b", 1)])),
        ("web-c", timeline([pod_line("web-c", 2)])),
    ]);
    let view = fx.open_web();
    assert_eq!(fx.read(&view, |v| v.skipped_pods()), 2);
    assert_eq!(fx.read(&view, |v| v.sources().len()), 1);
    assert!(fx.drawn("log-banner"));

    let service = fx.read(&view, |v| v.deps.service.clone());
    service.set_max_streams(5);
    fx.pass(Duration::from_secs(3));
    assert_eq!(fx.read(&view, |v| v.skipped_pods()), 0);
    assert_eq!(fx.read(&view, |v| v.sources().len()), 3);
}

#[gpui::test]
fn a_deployment_with_no_pods_says_so(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    fx.seed_web(vec![]);
    let view = fx.open_web();
    let notice = fx.read(&view, |v| v.no_pods_notice());
    assert_eq!(notice.as_deref(), Some("No pods match app=web"));
    assert!(fx.drawn("log-banner"));
}

#[gpui::test]
fn pods_that_have_not_started_are_waited_for_not_denied(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    fx.seed_web(vec![]);
    fx.ports.resources.insert(
        oxikube_testkit::pod()
            .name("web-a")
            .namespace("shop")
            .uid("uid-web-a")
            .label("app", "web")
            .container_creating()
            .build(),
    );
    let view = fx.open_web();
    assert_eq!(fx.read(&view, |v| v.no_pods_notice()), None);
    let waiting = fx.read(&view, |v| v.waiting_pods_notice());
    assert_eq!(waiting.as_deref(), Some("Waiting for 1 pod to start"));
    assert!(fx.drawn("log-banner"));
}

#[gpui::test]
fn a_deployment_that_cannot_be_read_is_a_failed_state_row(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    // No deployment in the cluster.
    let view = fx.open_web();
    fx.read(&view, |view| {
        assert!(matches!(view.line_window().state(), LogState::Failed(_)));
        let text = view.row_text(0).expect("the state row");
        assert!(text.starts_with("Not found:"), "{text}");
    });
}

#[gpui::test]
fn closing_the_tab_cancels_the_watch_and_every_stream(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    fx.seed_web(vec![
        ("web-a", timeline([pod_line("web-a", 0)])),
        ("web-b", timeline([pod_line("web-b", 1)])),
    ]);
    let view = fx.open_web();
    assert_eq!(fx.ports.logs.live_streams(), 2);
    assert_eq!(fx.ports.resources.live_watches(), 1);

    fx.vcx.update(|window, cx| {
        view.update(cx, |view, cx| view.on_close(window, cx));
    });
    fx.settle();
    assert_eq!(fx.ports.logs.live_streams(), 0, "a stream outlived the tab");
    assert_eq!(
        fx.ports.resources.live_watches(),
        0,
        "the pod watch outlived the tab"
    );
}

#[gpui::test]
fn previous_and_range_changes_reopen_the_merged_streams(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    fx.seed_web(vec![("web-a", timeline([pod_line("web-a", 0)]))]);
    let view = fx.open_web();
    assert_eq!(fx.opened().len(), 1);

    fx.script(timeline([pod_line("web-a", 9)]));
    fx.keys("2");
    fx.pass(Duration::from_millis(800));
    let opened = fx.opened();
    assert_eq!(opened.len(), 2, "the range key reopened the merged streams");
    assert_eq!(opened[1].since, Some(oxikube_ports::LogSince::Seconds(60)));
    assert_eq!(rows(&mut fx, &view), ["a INFO web-a line 9"]);
}

#[gpui::test]
fn what_is_switched_off_stays_off_when_the_streams_reopen(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    fx.seed_web(vec![
        ("web-a", timeline([pod_line("web-a", 0)])),
        ("web-b", timeline([pod_line("web-b", 1)])),
    ]);
    let view = fx.open_web();
    fx.vcx.update(|_, cx| {
        view.update(cx, |view, cx| view.request_toggle_source("web-a", None, cx));
    });
    fx.pass(Duration::from_millis(100));
    assert_eq!(rows(&mut fx, &view), ["b INFO web-b line 1"]);

    // A key reopens both streams: the same two pods come back, web-a still switched off.
    fx.script(timeline([pod_line("web-a", 10)]));
    fx.script(timeline([pod_line("web-b", 11)]));
    fx.keys("2");
    fx.pass(Duration::from_millis(800));
    assert_eq!(fx.opened().len(), 4);
    assert!(fx.read(&view, |v| v.is_source_hidden("web-a", "app")));
    assert_eq!(rows(&mut fx, &view), ["b INFO web-b line 11"]);
}

#[gpui::test]
fn opening_something_with_no_pods_to_merge_opens_nothing(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    let views = fx.views.clone();
    let config_map = oxikube_domain::ids::ResourceRef::namespaced(
        super::fixture::cluster(),
        oxikube_domain::ids::Gvk::new("", "v1", "ConfigMap"),
        "shop",
        "settings",
    );
    let opened = fx.vcx.update(|window, cx| {
        views.update(cx, |views, cx| {
            views.open(&config_map, &crate::view::OpenLogs::default(), window, cx)
        })
    });
    assert!(opened.is_none());
    let tabs = fx
        .vcx
        .update(|_, cx| fx.workspace.read(cx).items_of_type::<crate::LogView>());
    assert!(tabs.is_empty());
}
