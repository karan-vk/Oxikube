//! The field-selector fallback behind the node detail.

use futures::FutureExt as _;
use oxikube_ports::ListPage;
use oxikube_testkit::{FakeResourcePort, ResourceCall, pod};

use crate::nodes::list_node_pods;

#[test]
fn it_lists_with_the_node_selector_and_sums_the_demand_of_unfinished_pods() {
    let resources = FakeResourcePort::new();
    let on = |name: &str, node: &str| pod().namespace("x").name(name).node(node).running().build();
    // The fake does not evaluate `spec.nodeName`; script what a server would answer, including a
    // pod the selector should not have returned (another node) and a finished one.
    resources.script().list.push_ok(ListPage::complete(vec![
        on("b", "n1"),
        on("a", "n1"),
        on("other", "n2"),
        pod()
            .namespace("x")
            .name("done")
            .node("n1")
            .succeeded()
            .build(),
    ]));
    let found = list_node_pods(&resources, "n1")
        .now_or_never()
        .unwrap()
        .unwrap();
    let names: Vec<_> = found.pods.iter().map(|p| p.meta.name.to_string()).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(found.demand.pods, 2);
    assert_eq!(found.demand.cpu_requests, 200);

    let calls = resources.recorded_calls();
    let [
        ResourceCall::List {
            namespace, options, ..
        },
    ] = calls.as_slice()
    else {
        panic!("one list call, got {calls:?}");
    };
    assert_eq!(*namespace, None, "all namespaces");
    assert_eq!(
        options.field_selector.as_deref(),
        Some("spec.nodeName=n1,status.phase!=Succeeded,status.phase!=Failed")
    );
}

#[test]
fn it_follows_continue_tokens() {
    let resources = FakeResourcePort::new();
    let first = ListPage {
        items: vec![pod().namespace("x").name("a").node("n1").build()],
        continue_token: Some("t".into()),
        resource_version: None,
        remaining_item_count: None,
    };
    resources.script().list.push_ok(first);
    resources.script().list.push_ok(ListPage::complete(vec![
        pod().namespace("x").name("b").node("n1").build(),
    ]));
    let found = list_node_pods(&resources, "n1")
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(found.pods.len(), 2);
    let calls = resources.recorded_calls();
    assert_eq!(calls.len(), 2);
}

#[test]
fn a_forbidden_list_is_the_servers_error() {
    let resources = FakeResourcePort::new();
    resources
        .script()
        .list
        .push_err(oxikube_domain::OxiError::forbidden("pods is forbidden"));
    let err = list_node_pods(&resources, "n1")
        .now_or_never()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind(), oxikube_domain::ErrorKind::Forbidden);
}
