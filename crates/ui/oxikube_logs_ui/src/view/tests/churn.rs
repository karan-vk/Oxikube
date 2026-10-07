//! Reconnect and churn following (E08-S07) in the view: a replaced pod offers "follow
//! replacement" and the tab switches to the new pod; a broken stream says `Reconnecting n/m`; a
//! failed one offers "Reconnect" and keeps its lines; a multi-pod view lists the streams that
//! reconnect.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_app::logs::{Backoff, EndReason, LogConfig, LogState, ReconnectPolicy, SourceState};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_domain::log::LogLine;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_testkit::{LogCall, Timeline};
use oxikube_workspace::Item as _;
use serde_json::json;

use super::fixture::{Fx, line, lines, pod, pod_line};
use crate::view::Recovery;

/// `shop/<name>` as the fixture's pod (containers `app` and `metrics`), labelled `app=web` and
/// owned by the ReplicaSet `rs` of the `web` Deployment.
fn replica(name: &str, rs: &str) -> Resource {
    let mut json = pod().json;
    json["metadata"]["name"] = json!(name);
    json["metadata"]["uid"] = json!(format!("uid-{name}"));
    json["metadata"]["labels"] = json!({"app": "web"});
    json["metadata"]["ownerReferences"] = json!([{
        "apiVersion": "apps/v1", "kind": "ReplicaSet", "name": rs,
        "uid": format!("uid-{rs}"), "controller": true
    }]);
    Resource::from_json(json).unwrap()
}

/// The ReplicaSet `name` of the `web` Deployment.
fn replica_set(name: &str) -> Resource {
    let mut json = oxikube_testkit::replicaset()
        .name(name)
        .namespace("shop")
        .label("app", "web")
        .json();
    json["metadata"]["ownerReferences"] = json!([{
        "apiVersion": "apps/v1", "kind": "Deployment", "name": "web",
        "uid": "uid-web", "controller": true
    }]);
    Resource::from_json(json).unwrap()
}

/// Lines `0..n` now, then the end of the stream at `end` (the pod's container stopping).
fn ending(n: usize, end: Duration) -> Timeline<LogLine> {
    lines(0, n)
        .into_iter()
        .fold(Timeline::new(), |t, l| t.ok_at(Duration::ZERO, l))
        .ok_at(end, line(n))
}

/// Lines `0..n` now, then a dropped connection at 100 ms.
fn breaking(n: usize) -> Timeline<LogLine> {
    lines(0, n)
        .into_iter()
        .fold(Timeline::new(), |t, l| t.ok_at(Duration::ZERO, l))
        .err_at(
            Duration::from_millis(100),
            OxiError::network("connection reset by peer"),
        )
}

fn pods_opened(fx: &Fx) -> Vec<String> {
    fx.ports
        .logs
        .recorded_calls()
        .into_iter()
        .map(|LogCall::StreamLogs { pod, .. }| pod)
        .collect()
}

#[gpui::test]
fn a_replaced_pod_offers_to_follow_its_replacement_and_the_tab_switches_to_it(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::new(cx);
    let resources = fx.ports.resources.clone();
    resources.insert(
        oxikube_testkit::deployment()
            .name("web")
            .namespace("shop")
            .build(),
    );
    resources.insert(replica_set("web-5d8"));
    resources.insert(replica_set("web-7f9"));
    resources.insert(replica("web-0", "web-5d8"));
    let view = fx.open(ending(2, Duration::from_millis(500)));
    assert_eq!(fx.read(&view, |v| v.recovery()), None, "streaming");

    // The rollout: the pod is deleted, a pod of the new ReplicaSet runs; the old stream ends.
    let pod_kind = Gvk::new("", "v1", "Pod");
    assert!(resources.remove(&pod_kind, Some("shop"), "web-0"));
    resources.insert(replica("web-7f9-x", "web-7f9"));
    fx.pass(Duration::from_millis(600));
    fx.read(&view, |view| {
        assert_eq!(
            view.line_window().state(),
            &LogState::Ended(EndReason::PodReplaced)
        );
        assert_eq!(view.recovery(), Some(Recovery::FollowReplacement));
        let row = view.row_text(view.line_window().row_count() - 1).unwrap();
        assert!(row.starts_with("Pod replaced"), "{row}");
    });
    fx.draw();
    assert!(fx.drawn("log-recovery"));
    assert!(fx.drawn("log-follow-replacement"));

    // Follow it (the button sends `logs::FollowReplacement`; `shift-r` does the same).
    fx.script(Timeline::immediate([pod_line("web-7f9-x", 10)]).keep_open());
    fx.click("log-follow-replacement");
    fx.settle();
    assert!(
        fx.dispatcher
            .sent()
            .iter()
            .any(|c| matches!(c, Command::LogsFollowReplacement { .. }))
    );
    assert_eq!(pods_opened(&fx), ["web-0", "web-7f9-x"]);
    let title = fx.vcx.update(|_, cx| view.read(cx).tab_content(cx).title);
    assert_eq!(
        title.as_ref(),
        "web-7f9-x/app",
        "same container, the new pod"
    );
    fx.read(&view, |view| {
        assert_eq!(&*view.target().name, "web-7f9-x");
        assert_eq!(view.line_window().state(), &LogState::Streaming);
        assert_eq!(view.row_text(0).as_deref(), Some("INFO web-7f9-x line 10"));
        assert_eq!(view.recovery(), None);
    });
    fx.draw();
    assert!(!fx.drawn("log-recovery"));
}

#[gpui::test]
fn with_no_replacement_yet_the_tab_stays_and_says_so(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let resources = fx.ports.resources.clone();
    resources.insert(replica_set("web-5d8"));
    resources.insert(replica("web-0", "web-5d8"));
    let view = fx.open(ending(1, Duration::from_millis(300)));
    resources.remove(&Gvk::new("", "v1", "Pod"), Some("shop"), "web-0");
    fx.pass(Duration::from_millis(400));
    fx.keys("shift-r");
    assert_eq!(pods_opened(&fx), ["web-0"], "nothing to switch to");
    fx.read(&view, |view| {
        assert_eq!(&*view.target().name, "web-0");
        assert_eq!(view.recovery(), Some(Recovery::FollowReplacement));
    });
}

#[gpui::test]
fn a_broken_stream_says_reconnecting_and_comes_back_without_duplicates(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(breaking(3));
    fx.pass(Duration::from_millis(150));
    fx.read(&view, |view| {
        let state = view.line_window().state().clone();
        assert!(
            matches!(
                state,
                LogState::Reconnecting {
                    attempt: 1,
                    max: 5,
                    ..
                }
            ),
            "{state:?}"
        );
        let row = view.row_text(3).unwrap();
        assert!(row.starts_with("Reconnecting (1/5)"), "{row}");
        assert_eq!(view.recovery(), None, "it reconnects by itself");
    });
    // The reopened stream replays the overlap (lines 1 and 2) before the new lines.
    fx.script(Timeline::immediate(lines(1, 4)).keep_open());
    fx.pass(Duration::from_millis(800));
    fx.read(&view, |view| {
        assert_eq!(view.line_window().state(), &LogState::Streaming);
        assert_eq!(view.line_window().line_count(), 5, "lines 0..5, none twice");
        assert_eq!(view.row_text(4).as_deref(), Some("ERROR line 4"));
    });
}

#[gpui::test]
fn a_failed_stream_offers_reconnect_which_keeps_its_lines(cx: &mut TestAppContext) {
    let mut fx = Fx::with_config(
        cx,
        LogConfig {
            reconnect: ReconnectPolicy::Backoff(Backoff {
                max_retries: 0,
                ..Backoff::default()
            }),
            ..LogConfig::default()
        },
    );
    let view = fx.open(breaking(3));
    fx.pass(Duration::from_millis(150));
    fx.read(&view, |view| {
        assert!(matches!(view.line_window().state(), LogState::Failed(_)));
        assert_eq!(view.recovery(), Some(Recovery::Reconnect));
        assert_eq!(view.line_window().line_count(), 3, "the lines stay");
    });
    fx.draw();
    assert!(fx.drawn("log-reconnect"));

    fx.script(Timeline::immediate(lines(1, 4)).keep_open());
    fx.keys("r");
    assert!(
        fx.dispatcher
            .sent()
            .iter()
            .any(|c| matches!(c, Command::LogsReconnect { .. }))
    );
    fx.read(&view, |view| {
        assert_eq!(view.line_window().state(), &LogState::Streaming);
        assert_eq!(
            view.line_window().line_count(),
            5,
            "kept, and the overlap not twice"
        );
        assert_eq!(view.row_text(0).as_deref(), Some("INFO line 0"));
    });
    let reopened = &fx.opened()[1];
    assert!(reopened.since.is_some() && reopened.tail_lines.is_none());
}

#[gpui::test]
fn a_multi_pod_view_lists_the_streams_that_reconnect(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    let broken: OxiResult<LogLine> = Err(OxiError::network("connection reset by peer"));
    fx.seed_web(vec![
        (
            "web-a",
            Timeline::new()
                .ok_at(Duration::ZERO, pod_line("web-a", 0))
                .at(Duration::from_millis(900), broken),
        ),
        (
            "web-b",
            Timeline::immediate([pod_line("web-b", 1)]).keep_open(),
        ),
    ]);
    let view = fx.open_web();
    fx.pass(Duration::from_millis(200));
    fx.read(&view, |view| {
        let states: Vec<_> = view.sources().iter().map(|s| s.state.clone()).collect();
        assert_eq!(
            states,
            [
                SourceState::Reconnecting { attempt: 1, max: 5 },
                SourceState::Streaming
            ]
        );
        assert_eq!(view.line_window().state(), &LogState::Streaming);
    });
    fx.draw();
    assert!(
        fx.drawn("log-banner"),
        "the banner lists web-a/app reconnecting (1/5)"
    );
}
