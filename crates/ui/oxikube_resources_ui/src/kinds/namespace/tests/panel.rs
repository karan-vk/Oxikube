//! The Namespace panel in the detail drawer.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use serde_json::json;

use super::{ns, ns_ref};
use crate::detail::tests::fixture::Detail;
use crate::table::tests::fixture::cluster;

fn terminating() -> oxikube_domain::Resource {
    let mut value = ns(
        "gone",
        json!({
            "deletionTimestamp": "2026-01-02T03:04:05Z",
            "finalizers": ["example.com/hold"],
        }),
    )
    .to_value();
    value["status"] = json!({
        "phase": "Terminating",
        "conditions": [
            { "type": "NamespaceContentRemaining", "status": "True", "message": "Some resources are remaining: pods" },
        ],
    });
    oxikube_domain::Resource::from_json(value).unwrap()
}

#[gpui::test]
fn the_panel_shows_the_phase_and_a_use_button(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [ns("team-a", json!({}))]);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let view = d.open(&ns_ref("team-a"));
    assert!(d.shown("detail-kind-panel"));
    assert!(d.shown("namespace-panel"));
    assert!(d.shown("namespace-phase"));
    assert!(!d.shown("namespace-terminating"));
    assert!(d.shown("detail-overview"), "the generic rows stay");
    assert!(d.read(&view, |v| v.kind_panel().is_some()));

    d.f.dispatcher.clear();
    d.click("namespace-use");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::NamespaceSelect {
            cluster: cluster(),
            namespaces: vec!["team-a".into()],
        }]
    );
}

#[gpui::test]
fn a_terminating_namespace_says_what_holds_it(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [terminating()]);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let _view = d.open(&ns_ref("gone"));
    assert!(d.shown("namespace-terminating"));
    assert!(
        d.shown("namespace-finalizers"),
        "the finalizers that hold it"
    );
    assert!(
        d.shown("namespace-blocker-0"),
        "and what the controller reports"
    );
}

#[gpui::test]
fn a_system_namespace_carries_its_warning(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [ns("kube-system", json!({}))]);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let _view = d.open(&ns_ref("kube-system"));
    assert!(d.shown("namespace-protected"));
}

#[gpui::test]
fn an_hnc_subnamespace_is_named(cx: &mut TestAppContext) {
    let mut d = Detail::new(
        cx,
        [ns(
            "team-a-dev",
            json!({ "annotations": { "hnc.x-k8s.io/subnamespace-of": "team-a" } }),
        )],
    );
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let _view = d.open(&ns_ref("team-a-dev"));
    assert!(d.shown("namespace-hnc"));
}
