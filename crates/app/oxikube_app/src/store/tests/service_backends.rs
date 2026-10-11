//! A Service's detail over the store (E12-S13): one EndpointSlice query by
//! `kubernetes.io/service-name` and one pod query by the Service's selector, both scoped to the
//! Service's namespace and filtered by the server, and nothing else.

use oxikube_domain::view::ServiceSummary;
use oxikube_ports::WatchOptions;
use oxikube_testkit::{ResourceCall, resource};
use serde_json::json;

use super::*;
use crate::network::{PodsPlan, ServiceBackends};

fn service() -> ServiceSummary {
    let res = resource("v1", "Service")
        .name("web")
        .namespace("demo")
        .field(
            "spec",
            json!({"selector": {"app": "web", "tier": "front"}, "ports": [{"port": 80}]}),
        )
        .build();
    ServiceSummary::from_resource(&res).unwrap()
}

fn slice(ns: &str, name: &str, service: &str) -> Resource {
    let mut r = resource("discovery.k8s.io/v1", "EndpointSlice")
        .name(name)
        .namespace(ns)
        .label("kubernetes.io/service-name", service)
        .field("addressType", json!("IPv4"))
        .field("endpoints", json!([{"addresses": ["10.0.0.1"]}]))
        .build();
    r.meta.resource_version = Some("1".into());
    r
}

fn labelled(ns: &str, name: &str, app: &str, tier: &str) -> Resource {
    let mut r = pod()
        .namespace(ns)
        .name(name)
        .label("app", app)
        .label("tier", tier)
        .build();
    r.meta.resource_version = Some("1".into());
    r
}

fn requests(h: &Harness) -> Vec<(String, Option<String>, Option<String>)> {
    h.resources
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Watch {
                kind,
                namespace,
                options: WatchOptions { label_selector, .. },
            } => Some((kind.kind.to_string(), namespace, label_selector)),
            ResourceCall::List {
                kind, namespace, ..
            } => panic!("a detail never lists locally: {kind:?} in {namespace:?}"),
            _ => None,
        })
        .collect()
}

#[test]
fn the_detail_issues_one_slice_query_and_one_pod_query_with_server_side_selectors() {
    let mut h = Harness::with_objects([
        slice("demo", "web-abc", "web"),
        slice("demo", "web-def", "web"),
        slice("demo", "api-xyz", "api"),
        slice("other", "web-ghi", "web"),
        labelled("demo", "web-1", "web", "front"),
        labelled("demo", "web-2", "web", "front"),
        labelled("demo", "web-back", "web", "back"),
        labelled("demo", "db-0", "db", "front"),
        labelled("other", "web-elsewhere", "web", "front"),
    ]);
    let backends = ServiceBackends::of(&service());
    let PodsPlan::Query(pod_query) = backends.pods else {
        panic!("the Service selects pods");
    };

    let mut slices = h.subscribe(backends.slices);
    let mut pods = h.subscribe(pod_query);
    let (mut slice_rows, mut pod_rows) = (Mirror::default(), Mirror::default());
    slice_rows.drain(&mut slices);
    pod_rows.drain(&mut pods);

    assert_eq!(slice_rows.names(), ["demo/web-abc", "demo/web-def"]);
    assert_eq!(pod_rows.names(), ["demo/web-1", "demo/web-2"]);
    assert_eq!(
        requests(&h),
        [
            (
                "EndpointSlice".to_owned(),
                Some("demo".to_owned()),
                Some("kubernetes.io/service-name=web".to_owned())
            ),
            (
                "Pod".to_owned(),
                Some("demo".to_owned()),
                Some("app=web,tier=front".to_owned())
            ),
        ],
        "exactly two queries, each namespaced and carrying its selector to the server"
    );
}

#[test]
fn closing_the_detail_releases_both_feeds() {
    let mut h = Harness::with_options(options_with_grace(0));
    let backends = ServiceBackends::of(&service());
    let PodsPlan::Query(pod_query) = backends.pods else {
        panic!("the Service selects pods");
    };
    let slices = h.subscribe(backends.slices);
    let pods = h.subscribe(pod_query);
    assert_eq!(h.resources.live_watches(), 2);
    drop((slices, pods));
    h.advance(1);
    assert_eq!(h.resources.live_watches(), 0, "no feed outlives the detail");
}
