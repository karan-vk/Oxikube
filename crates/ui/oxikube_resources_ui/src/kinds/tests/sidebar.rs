//! The cluster sidebar built from the registry, over the testkit's discovery sets: a stock
//! cluster, one with the Gateway API, one whose discovery lists only CRDs.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_domain::access::AccessRules;
use oxikube_domain::kinds::ResourceKind;
use oxikube_testkit::discovery_sets;

use super::row_ids;
use crate::table::tests::fixture::{Fixture, cluster};

/// A window with the built-in kinds registered, connected to a cluster serving `kinds` to a user
/// who may list everything.
fn connected(cx: &mut TestAppContext, kinds: Vec<ResourceKind>) -> Fixture {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.access.set_rules(AccessRules::all_access());
    ports.discovery.set_kinds(kinds);
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.settle();
    f
}

fn sections(ids: &[String]) -> Vec<&str> {
    ids.iter()
        .map(String::as_str)
        .filter(|id| {
            !id.contains('/')
                && !id.starts_with("crd:")
                && !matches!(*id, "access-limited" | "access-pending" | "access-warning")
        })
        .collect()
}

#[gpui::test]
fn the_sidebar_renders_the_registered_sections_of_a_stock_cluster(cx: &mut TestAppContext) {
    let mut f = connected(cx, discovery_sets::core());
    let ids = row_ids(&mut f);
    assert_eq!(
        sections(&ids),
        [
            "cluster",
            "nodes",
            "workloads",
            "config",
            "network",
            "storage",
            "namespaces",
            "events",
            "helm",
            "access-control"
        ]
    );
    for entry in [
        "workloads/pods",
        "workloads/replicationcontrollers",
        "config/limitranges",
        "config/mutatingwebhookconfigurations",
        "network/endpointslices",
        "storage/storageclasses",
    ] {
        assert!(ids.contains(&entry.to_owned()), "{entry} in {ids:?}");
    }
    // Optional APIs the stock cluster does not serve are absent, and that is not "limited
    // access".
    for absent in [
        "network/gateways",
        "storage/volumesnapshots",
        "config/verticalpodautoscalers",
    ] {
        assert!(!ids.contains(&absent.to_owned()), "{absent} in {ids:?}");
    }
    assert!(!ids.contains(&"access-limited".to_owned()));
}

#[gpui::test]
fn the_gateway_api_shows_under_network_when_served(cx: &mut TestAppContext) {
    let mut f = connected(cx, discovery_sets::core_with_gateway_api());
    let ids = row_ids(&mut f);
    for entry in [
        "network/gatewayclasses",
        "network/gateways",
        "network/httproutes",
        "network/grpcroutes",
    ] {
        assert!(ids.contains(&entry.to_owned()), "{entry} in {ids:?}");
    }
    // TLSRoute is not in the fixture's Gateway API (it is experimental): still absent.
    assert!(!ids.contains(&"network/tlsroutes".to_owned()));
    let network: Vec<&String> = ids.iter().filter(|id| id.starts_with("network/")).collect();
    assert_eq!(
        network.first().map(|s| s.as_str()),
        Some("network/services")
    );
}

#[gpui::test]
fn a_crd_only_discovery_shows_custom_resources(cx: &mut TestAppContext) {
    let mut f = connected(cx, discovery_sets::crds_only());
    let ids = row_ids(&mut f);
    let shown = sections(&ids);
    assert!(shown.contains(&"custom-resources"), "{ids:?}");
    assert!(ids.contains(&"crd:cert-manager.io".to_owned()), "{ids:?}");
    // No built-in kind is served: their sections are gone (Cluster needs nothing; the
    // workspace's placeholders without a registered kind keep their own rules).
    for gone in [
        "nodes",
        "workloads",
        "config",
        "network",
        "storage",
        "namespaces",
    ] {
        assert!(!shown.contains(&gone), "{gone} in {shown:?}");
    }
}
