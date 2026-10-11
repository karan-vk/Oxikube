//! The Endpoints and EndpointSlice panels (E12-S13): endpoints with readiness, the node and the
//! pod behind each, a way back to the Service, and no feed of their own.

use gpui::{Entity, TestAppContext};
use oxikube_app::columns::Tone;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_testkit::resource;
use serde_json::json;

use super::EndpointsPanel;
use super::model::EndpointsView;
use crate::detail::DetailView;
use crate::detail::tests::fixture::Detail;
use crate::table::tests::fixture::cluster;

fn slice() -> Resource {
    let mut r = resource("discovery.k8s.io/v1", "EndpointSlice")
        .name("web-abc")
        .namespace("shop")
        .label("kubernetes.io/service-name", "web")
        .field("addressType", json!("IPv4"))
        .field(
            "ports",
            json!([{"name": "http", "port": 8080, "protocol": "TCP"}]),
        )
        .field(
            "endpoints",
            json!([
                {"addresses": ["10.1.0.5"], "conditions": {"ready": true}, "nodeName": "n1",
                 "targetRef": {"kind": "Pod", "name": "web-0", "namespace": "shop"}},
                {"addresses": ["10.1.0.6"], "conditions": {"ready": false}},
                {"addresses": ["10.1.0.7"], "conditions": {"ready": false, "terminating": true}},
            ]),
        )
        .build();
    r.meta.resource_version = Some("1".into());
    r
}

fn endpoints() -> Resource {
    let mut r = resource("v1", "Endpoints")
        .name("web")
        .namespace("shop")
        .field(
            "subsets",
            json!([{
                "addresses": [{"ip": "10.1.0.5", "targetRef": {"kind": "Pod", "name": "web-0"}}],
                "notReadyAddresses": [{"ip": "10.1.0.6"}],
                "ports": [{"port": 8080, "protocol": "TCP"}],
            }]),
        )
        .build();
    r.meta.resource_version = Some("1".into());
    r
}

fn detail(cx: &mut TestAppContext, objects: Vec<Resource>) -> Detail {
    let mut d = Detail::new(cx, objects);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    d
}

fn target(group: &str, version: &str, kind: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new(group, version, kind), "shop", name)
}

fn view_of(d: &mut Detail, view: &Entity<DetailView>) -> EndpointsView {
    d.draw();
    let any = d
        .read(view, |v| v.kind_panel().cloned())
        .expect("the kind has its own panel");
    let panel = any
        .downcast::<EndpointsPanel>()
        .expect("an endpoints panel");
    d.f.vcx
        .update(|_, cx| panel.read(cx).view().cloned())
        .expect("built")
}

#[test]
fn a_slice_view_has_readiness_and_links() {
    let cluster = cluster();
    let view = EndpointsView::of(&slice(), &cluster).expect("a slice");
    assert_eq!(view.address_type, "IPv4");
    assert_eq!(view.ports, "8080/TCP");
    assert_eq!(view.counts.summary(), "1 ready, 2 not ready");
    let states: Vec<(&str, Tone)> = view.rows.iter().map(|r| (r.state, r.tone)).collect();
    assert_eq!(
        states,
        [
            ("Ready", Tone::Ok),
            ("Not ready", Tone::Warn),
            ("Terminating", Tone::Warn)
        ]
    );
    assert_eq!(view.rows[0].node, "n1");
    assert_eq!(
        view.rows[0]
            .target
            .as_ref()
            .map(|(name, r)| (name.as_str(), r.clone())),
        Some(("web-0", target("", "v1", "Pod", "web-0")))
    );
    assert_eq!(view.service, Some(target("", "v1", "Service", "web")));
}

#[test]
fn an_endpoints_view_is_named_after_its_service() {
    let view = EndpointsView::of(&endpoints(), &cluster()).expect("endpoints");
    assert_eq!(view.service, Some(target("", "v1", "Service", "web")));
    assert_eq!(view.counts.summary(), "1 ready, 1 not ready");
    assert_eq!(view.ports, "8080/TCP");
    assert_eq!(view.rows[1].state, "Not ready");
    assert_eq!(
        view.rows[0].target.as_ref().map(|(_, r)| r.clone()),
        Some(target("", "v1", "Pod", "web-0")),
        "a target without a namespace is in the object's"
    );
    assert!(
        EndpointsView::of(
            &Resource::from_json(json!({
                "apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "c"}
            }))
            .unwrap(),
            &cluster()
        )
        .is_none()
    );
}

#[test]
fn a_long_slice_is_capped_and_still_counted() {
    let many: Vec<_> = (0..250)
        .map(|i| json!({"addresses": [format!("10.0.{}.{}", i / 250, i % 250)]}))
        .collect();
    let mut json = slice().to_value();
    json["endpoints"] = json!(many);
    let view = EndpointsView::of(&Resource::from_json(json).unwrap(), &cluster()).expect("a slice");
    assert_eq!(view.rows.len(), super::model::MAX_ENDPOINTS);
    assert_eq!(view.more, 150);
    assert_eq!(view.counts.total(), 250);
}

#[gpui::test]
fn an_endpoint_slice_detail_draws_its_endpoints(cx: &mut TestAppContext) {
    let mut d = detail(cx, vec![slice()]);
    let view = d.open(&target(
        "discovery.k8s.io",
        "v1",
        "EndpointSlice",
        "web-abc",
    ));
    for selector in [
        "endpoints-panel",
        "endpoints-service",
        "endpoint-0",
        "endpoint-1",
        "endpoint-2",
        "detail-overview",
    ] {
        assert!(d.shown(selector), "{selector} is on screen");
    }
    assert_eq!(view_of(&mut d, &view).rows.len(), 3);

    d.f.dispatcher.clear();
    d.click("endpoint-0");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: target("", "v1", "Pod", "web-0")
        }],
        "an endpoint opens its pod"
    );
}

#[gpui::test]
fn an_endpoints_detail_links_back_to_the_service(cx: &mut TestAppContext) {
    let mut d = detail(cx, vec![endpoints()]);
    d.open(&target("", "v1", "Endpoints", "web"));
    assert!(d.shown("endpoints-panel"));
    d.f.dispatcher.clear();
    d.click("endpoints-service");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: target("", "v1", "Service", "web")
        }]
    );
}

#[gpui::test]
fn the_panels_start_no_feed_of_their_own(cx: &mut TestAppContext) {
    let mut d = detail(cx, vec![slice()]);
    d.open(&target(
        "discovery.k8s.io",
        "v1",
        "EndpointSlice",
        "web-abc",
    ));
    assert_eq!(
        d.f.ports().resources.live_watches(),
        1,
        "only the object's own watch"
    );
}
