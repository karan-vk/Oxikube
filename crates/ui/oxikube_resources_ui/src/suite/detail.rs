//! The detail drawer end to end: a row opened from the table, its four tabs (Overview, YAML,
//! Describe, Events) and a live feed changing the object under them.

use oxikube_domain::Resource;
use oxikube_ports::{DescribeOutput, DescribeSource};
use oxikube_testkit::{ScriptedFeed, TICK};
use serde_json::json;

use crate::detail::tests::fixture::{Detail, edited, kind, pod_ref, web_pod, web_replicaset};
use crate::detail::{DetailState, DetailTab, Mount};

fn event(name: &str, pod: &str, reason: &str, at: &str) -> Resource {
    edited(
        oxikube_testkit::resource("v1", "Event")
            .namespace("shop")
            .name(name)
            .field("type", json!("Warning"))
            .field("reason", json!(reason))
            .field("message", json!(format!("{reason} {pod}")))
            .field("lastTimestamp", json!(at))
            .field(
                "involvedObject",
                json!({"apiVersion": "v1", "kind": "Pod", "name": pod,
                       "namespace": "shop", "uid": format!("u-{pod}")}),
            )
            .build(),
        |_| {},
    )
}

fn pods() -> oxikube_domain::kinds::ResourceKind {
    kind("", "v1", "Pod", "pods", true)
}

#[gpui::test]
fn enter_on_a_row_opens_its_detail_in_the_drawer(cx: &mut gpui::TestAppContext) {
    let mut d = Detail::new(cx, [web_pod(), web_replicaset()]);
    let table = d.f.open(pods());
    assert!(d.drawer().is_none(), "no drawer until something is opened");
    d.f.keys(&table, "j enter");
    d.settle();
    let view = d.drawer_view().expect("enter opened the drawer");
    assert_eq!(d.read(&view, |v| v.target().clone()), pod_ref("web-0"));
    assert_eq!(d.read(&view, |v| v.mount()), Mount::Drawer);
    assert_eq!(d.read(&view, |v| v.state().clone()), DetailState::Live);
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Overview);
    assert!(d.shown("detail-view"));
    assert!(d.shown("detail-tab-overview"));
    assert!(d.shown("detail-tab-yaml"));
    assert!(d.shown("detail-tab-describe"));
    assert!(d.shown("detail-tab-events"));
    let model = d.read(&view, |v| v.model().cloned()).expect("a model");
    assert_eq!(&*model.header.name, "web-0");
    assert_eq!(
        model.header.status.as_ref().map(|c| c.text.as_str()),
        Some("Running")
    );
}

#[gpui::test]
fn every_tab_shows_its_content_and_follows_the_live_object(cx: &mut gpui::TestAppContext) {
    let pod = web_pod();
    let renamed = edited(pod.clone(), |json| {
        json["metadata"]["resourceVersion"] = json!("8");
        json["metadata"]["labels"]["app"] = json!("web-v2");
    });
    let feed = ScriptedFeed::new().initial([pod]).modify(1, renamed);
    let mut d = Detail::new(
        cx,
        [
            event("web-0.backoff", "web-0", "BackOff", "2026-01-01T00:30:00Z"),
            event("web-1.started", "web-1", "Started", "2026-01-01T00:31:00Z"),
        ],
    );
    feed.install(&d.f.ports().resources);
    d.f.ports()
        .describe
        .script()
        .describe
        .push_ok(DescribeOutput {
            text: "Name: web-0\nNamespace: shop".into(),
            source: DescribeSource::Native,
        });
    let table = d.f.open(pods());
    d.f.keys(&table, "j enter");
    d.settle();
    let view = d.drawer_view().expect("the drawer");

    // Overview: header, labels, conditions.
    let labels = |d: &mut Detail, v: &gpui::Entity<crate::detail::DetailView>| {
        d.read(v, |v| v.model().map(|m| m.labels[0].copy_text()))
    };
    assert_eq!(labels(&mut d, &view).as_deref(), Some("app=web"));
    assert!(d.shown("detail-condition-0"));

    // YAML: the object's text in a read-only editor.
    d.click("detail-tab-yaml");
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Yaml);
    let yaml = d
        .read(&view, |v| v.yaml().map(str::to_owned))
        .expect("YAML");
    assert!(
        yaml.contains("kind: Pod") && yaml.contains("app: web\n"),
        "{yaml}"
    );
    assert!(d.shown("detail-yaml-editor"));

    // Describe: what the describe port returned, named by its source.
    d.click("detail-tab-describe");
    assert_eq!(
        d.read(&view, |v| v.describe_text().map(str::to_owned))
            .as_deref(),
        Some("Name: web-0\nNamespace: shop")
    );
    assert!(d.shown("detail-describe-text"));

    // Events: only this pod's.
    d.click("detail-tab-events");
    d.settle();
    let reasons: Vec<String> = d.read(&view, |v| {
        v.event_rows()
            .iter()
            .map(|e| e.reason.to_string())
            .collect()
    });
    assert_eq!(reasons, ["BackOff"], "web-1's event is not shown");
    assert!(d.shown("detail-event-0"));

    // A new version of the object arrives: the Overview follows it at once.
    d.f.ports().resources.clock().advance(TICK);
    d.settle();
    assert_eq!(
        labels(&mut d, &view).as_deref(),
        Some("app=web-v2"),
        "overview"
    );
    // The YAML is made when its tab is shown: back on it, it is the new version.
    d.click("detail-tab-yaml");
    let yaml = d
        .read(&view, |v| v.yaml().map(str::to_owned))
        .expect("YAML");
    assert!(
        yaml.contains("app: web-v2"),
        "the YAML shows the new version: {yaml}"
    );
    d.click("detail-tab-overview");
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Overview);
    assert!(d.shown("detail-condition-0"));
}
