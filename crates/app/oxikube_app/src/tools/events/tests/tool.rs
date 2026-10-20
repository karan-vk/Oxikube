//! `k8s.get_events`: filters, limits, absence, on-demand start and redaction.

use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_ports::ToolContext;
use serde_json::json;

use super::*;
use crate::events::EventsState;

/// Four pods' worth of events across two namespaces; `web` has a Warning and a Normal.
fn fixture() -> Vec<Event> {
    vec![
        ev("e1", "Pod", "shop", "web-0", "uid-web", 10),
        warning("e2", "Pod", "shop", "web-0", "uid-web", 20, "BackOff"),
        ev("e3", "Pod", "shop", "db-0", "uid-db", 30),
        warning(
            "e4",
            "Pod",
            "kube-system",
            "dns-0",
            "uid-dns",
            40,
            "Unhealthy",
        ),
        ev("e5", "Deployment", "shop", "web", "uid-dep", 50),
    ]
}

#[tokio::test]
async fn it_returns_every_event_newest_first_when_asked_for_nothing() {
    let h = Harness::with_events(fixture()).await;
    let out = h.get_events(json!({})).await;
    assert!(!out.is_error);
    let s = structured(&out);
    assert_eq!(s["status"], "ok");
    assert_eq!(s["total"], 5);
    assert_eq!(s["returned"], 5);
    assert_eq!(s["truncated"], false);
    assert_eq!(s["cluster"], "dev");
    // Newest (by last seen) first.
    assert_eq!(
        reasons(&out),
        ["Pulled", "Unhealthy", "Pulled", "BackOff", "Pulled"]
    );
    let first = &s["events"][0];
    assert_eq!(first["object"], "Deployment/shop/web");
    assert_eq!(first["type"], "Normal");
    assert_eq!(first["count"], 1);
    assert_eq!(first["age"], "9m", "ten minutes after t0, seen at +50s");
    assert!(
        text(&out).contains("TYPE\tREASON\tOBJECT"),
        "{}",
        text(&out)
    );
}

#[tokio::test]
async fn it_filters_by_namespace() {
    let h = Harness::with_events(fixture()).await;
    let out = h.get_events(json!({"ns": "kube-system"})).await;
    assert_eq!(reasons(&out), ["Unhealthy"]);
    assert_eq!(structured(&out)["namespace"], "kube-system");
    let all = h.get_events(json!({"ns": "all"})).await;
    assert_eq!(structured(&all)["total"], 5);
    assert!(structured(&all)["namespace"].is_null());
}

#[tokio::test]
async fn it_filters_by_the_involved_object_by_kind_and_name_or_by_uid() {
    let h = Harness::with_events(fixture()).await;
    let by_name = h
        .get_events(json!({"involved": {"kind": "Pod", "name": "web-0"}}))
        .await;
    assert_eq!(reasons(&by_name), ["BackOff", "Pulled"]);
    // A kind word of the cluster's alias table works too.
    let by_alias = h
        .get_events(json!({"ns": "shop", "involved": {"kind": "deploy", "name": "web"}}))
        .await;
    assert_eq!(structured(&by_alias)["total"], 1);
    assert_eq!(
        structured(&by_alias)["events"][0]["object"],
        "Deployment/shop/web"
    );
    let by_uid = h.get_events(json!({"involved": {"uid": "uid-db"}})).await;
    assert_eq!(structured(&by_uid)["total"], 1);
    assert_eq!(structured(&by_uid)["events"][0]["object"], "Pod/shop/db-0");
}

#[tokio::test]
async fn it_finds_an_event_whose_writer_gave_no_uid_by_the_objects_name() {
    let mut nameless = ev("e6", "Pod", "shop", "web-0", "uid-web", 40);
    nameless.regarding_uid = None;
    let h = Harness::with_events([fixture(), vec![nameless]].concat()).await;
    // The name finds it: the writer left the involved uid out.
    let by_name = h
        .get_events(json!({"involved": {"kind": "Pod", "name": "web-0"}}))
        .await;
    assert_eq!(reasons(&by_name), ["Pulled", "BackOff", "Pulled"]);
    // A uid cannot: the event carries none, and kubectl's rule is the same.
    let by_uid = h.get_events(json!({"involved": {"uid": "uid-web"}})).await;
    assert_eq!(
        structured(&by_uid)["total"],
        2,
        "only the events that carry the uid"
    );
}

#[tokio::test]
async fn it_filters_by_since_as_a_duration_or_a_timestamp() {
    let h = Harness::with_events(fixture()).await;
    // Ten minutes after t0 is "now": 10m back is t0 itself, 9m back is t0 + 60s (all older).
    let all = h.get_events(json!({"since": "10m"})).await;
    assert_eq!(structured(&all)["total"], 5);
    let none = h.get_events(json!({"since": "9m"})).await;
    assert_eq!(structured(&none)["total"], 0);
    // By timestamp: events seen at or after t0 + 30s.
    let stamp = h.get_events(json!({"since": "2026-01-01T00:00:30Z"})).await;
    assert_eq!(structured(&stamp)["total"], 3);
    assert_eq!(structured(&stamp)["since"], "2026-01-01T00:00:30Z");
}

#[tokio::test]
async fn it_filters_by_type() {
    let h = Harness::with_events(fixture()).await;
    let warnings = h.get_events(json!({"type": "Warning"})).await;
    assert_eq!(reasons(&warnings), ["Unhealthy", "BackOff"]);
}

#[tokio::test]
async fn a_limit_caps_the_rows_and_says_so() {
    let h = Harness::with_events(fixture()).await;
    let out = h.get_events(json!({"limit": 2})).await;
    let s = structured(&out);
    assert_eq!(s["returned"], 2);
    assert_eq!(s["total"], 5);
    assert_eq!(s["truncated"], true);
    assert!(
        s["note"]
            .as_str()
            .expect("a note")
            .contains("narrow the query"),
        "{s}"
    );
    assert!(text(&out).contains("(truncated)"), "{}", text(&out));
}

#[tokio::test]
async fn the_default_limit_is_applied_and_counted() {
    let many: Vec<Event> = (0..45)
        .map(|i| ev(&format!("e{i}"), "Pod", "shop", "web-0", "uid-web", i))
        .collect();
    let h = Harness::with_events(many).await;
    let out = h.get_events(json!({})).await;
    let s = structured(&out);
    assert_eq!(s["returned"], super::super::DEFAULT_LIMIT);
    assert_eq!(s["total"], 45);
    assert_eq!(s["truncated"], true);
}

#[tokio::test]
async fn no_events_is_an_empty_result_with_a_note_not_an_error() {
    let h = Harness::with_events([]).await;
    let out = h.get_events(json!({})).await;
    assert!(!out.is_error);
    let s = structured(&out);
    assert_eq!(s["status"], "ok");
    assert_eq!(s["total"], 0);
    assert_eq!(s["events"], json!([]));
    assert!(
        s["note"]
            .as_str()
            .expect("a note")
            .contains("no events match")
    );
}

#[tokio::test]
async fn a_feed_nobody_started_is_started_by_the_call() {
    let h = Harness::with_events(fixture()).await;
    assert_eq!(h.state(), EventsState::Idle, "nothing subscribed yet");
    assert_eq!(h.port().live_feeds(), 0);
    let out = h.get_events(json!({})).await;
    assert_eq!(structured(&out)["total"], 5);
    assert_eq!(structured(&out)["feed"], "ready");
    assert_eq!(h.port().live_feeds(), 1, "one feed, opened by the call");
    // A second call shares it.
    h.get_events(json!({})).await;
    assert_eq!(h.port().recorded_calls().len(), 1);
}

#[tokio::test]
async fn a_cluster_that_is_not_connected_is_a_tool_error_the_model_sees() {
    let h = Harness::new();
    h.connect().await;
    h.sessions.disconnect(&cluster()).expect("disconnect");
    let out = h
        .tools
        .invoke(
            "k8s.get_events",
            json!({}),
            &ToolContext::agent().with_cluster(cluster()),
        )
        .await
        .expect("a result");
    assert!(out.is_error);
    assert!(text(&out).contains("not connected"), "{}", text(&out));
    assert_eq!(
        h.port().recorded_calls().len(),
        0,
        "no feed for a closed cluster"
    );
}

#[tokio::test]
async fn a_cluster_with_no_session_is_a_tool_error_too() {
    let h = Harness::new();
    let out = h
        .tools
        .invoke(
            "k8s.get_events",
            json!({}),
            &ToolContext::agent().with_cluster(cluster()),
        )
        .await
        .expect("a result");
    assert!(out.is_error);
    assert!(text(&out).contains("no session"), "{}", text(&out));
}

#[tokio::test]
async fn a_feed_that_failed_for_good_is_unavailable_with_the_reason() {
    let h = Harness::new();
    h.connect().await;
    h.port()
        .script()
        .watch_events
        .push_err(OxiError::forbidden("events is forbidden for this user"));
    let out = h.get_events(json!({})).await;
    assert!(!out.is_error, "absence is an answer, not an error");
    let s = structured(&out);
    assert_eq!(s["status"], "unavailable");
    assert!(
        s["reason"]
            .as_str()
            .expect("a reason")
            .contains("forbidden"),
        "{s}"
    );
    assert!(text(&out).starts_with("events unavailable on dev"));
}

#[tokio::test]
async fn secrets_in_messages_are_masked_and_long_messages_cut() {
    let mut leaky = ev("e1", "Pod", "shop", "web-0", "uid-web", 10);
    leaky.message =
        "Pulling image \"https://admin:hunter2@registry.example.com/web:1\"\nsecond line".into();
    let mut long = ev("e2", "Pod", "shop", "web-0", "uid-web", 20);
    long.message = "x".repeat(2_000);
    let h = Harness::with_events([leaky, long]).await;
    let out = h.get_events(json!({})).await;
    let all = format!("{} {}", text(&out), structured(&out));
    assert!(!all.contains("hunter2"), "{all}");
    let rows = structured(&out)["events"].as_array().expect("rows").clone();
    let cut = rows[0]["message"].as_str().expect("message");
    assert_eq!(
        cut.chars().count(),
        super::super::MAX_MESSAGE_CHARS + 1,
        "cut on a char boundary with an ellipsis"
    );
    assert!(cut.ends_with('…'));
    let masked = rows[1]["message"].as_str().expect("message");
    assert!(!masked.contains('\n'), "one line: {masked}");
}

#[tokio::test]
async fn bad_arguments_are_refused_before_anything_is_read() {
    let h = Harness::with_events(fixture()).await;
    let invoke = |args| async {
        h.tools
            .invoke("k8s.get_events", args, &ToolContext::agent())
            .await
            .expect_err("a malformed call")
    };
    for (args, needle) in [
        (json!({"since": "soon"}), "since"),
        (json!({"since": "30d"}), "since"),
        (json!({"limit": 0}), "at least 1"),
        (json!({"limit": 101}), "at most 100"),
        (json!({"limit": "ten"}), "integer"),
        (json!({"type": "Error"}), "one of"),
        (json!({"unknown": 1}), "unknown"),
        (json!({"involved": {"kind": "Pod"}}), "kind"),
        (json!({"involved": {"uid": "u", "name": "n"}}), "uid"),
        (json!({"ns": ""}), "at least 1"),
    ] {
        let error = invoke(args.clone()).await;
        assert_eq!(error.kind(), ErrorKind::Validation, "{args}");
        assert!(
            error.message().contains(needle),
            "{args}: {}",
            error.message()
        );
    }
    assert_eq!(h.port().recorded_calls().len(), 0, "no feed was started");
}
