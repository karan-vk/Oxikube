//! The kind registry's gate on the sidebar (E12-S01): an entry that lists a registered kind shows
//! only when the cluster serves the kind and the visibility hook allows it. Pure rules first, then
//! the panel following discovery.

use std::collections::BTreeMap;
use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_app::kinds::AllowAll;
use oxikube_app::{AccessOutcome, KindGate, KindVisibility, ServedKinds};
use oxikube_domain::access::AccessRules;
use oxikube_domain::ids::{ClusterId, Gvk, Scope};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{DiscoveryEvent, KindsChange};
use oxikube_testkit::discovery_sets;
use oxikube_ui::IconName;

use super::{Fixture, id};
use crate::sidebar::{
    AccessState, Row, RowInputs, SidebarEntry, SidebarRegistry, SidebarSection, build_rows,
};

fn gateway() -> Gvk {
    Gvk::new("gateway.networking.k8s.io", "v1", "Gateway")
}

/// A Network section as the kind registry publishes it: Services (every cluster) and Gateways
/// (Gateway API only), each gated on its kind; and a Gateway-only section.
fn sections() -> Vec<SidebarSection> {
    let services = SidebarEntry::kind("services", "Services", "", "services")
        .with_kind(Gvk::new("", "v1", "Service"), Scope::Namespaced);
    let gateways = SidebarEntry::kind(
        "gateways",
        "Gateways",
        "gateway.networking.k8s.io",
        "gateways",
    )
    .with_kind(gateway(), Scope::Namespaced);
    vec![
        SidebarSection::new("network", "Network", IconName::Network, 500)
            .with_entries([services, gateways.clone()]),
        SidebarSection::new("gateway-only", "Gateway", IconName::Network, 550)
            .with_entries([gateways]),
    ]
}

fn row_ids(served: Option<&ServedKinds>, visibility: &dyn KindVisibility) -> Vec<String> {
    let cluster = id("prod");
    let namespaces = NamespaceSelection::All;
    let access = AccessState::Known(AccessOutcome::Reviewed(AccessRules::all_access()));
    let sections = sections();
    build_rows(&RowInputs {
        sections: &sections,
        integrations: &[],
        custom: None,
        access: &access,
        crd_watch_forbidden: false,
        open: &BTreeMap::new(),
        custom_query: "",
        show_restricted: false,
        gate: Some(KindGate {
            cluster: &cluster,
            served,
            visibility,
            namespaces: &namespaces,
        }),
    })
    .iter()
    .map(|r| r.id().to_owned())
    .collect()
}

/// Hides Gateways.
struct NoGateways;

impl KindVisibility for NoGateways {
    fn is_listable(&self, _: &ClusterId, gvk: &Gvk, _: Option<&str>) -> Option<bool> {
        (&*gvk.kind == "Gateway").then_some(false)
    }
}

#[test]
fn a_kind_the_cluster_does_not_serve_is_absent_not_denied() {
    let core = ServedKinds::from_kinds(&discovery_sets::core());
    let ids = row_ids(Some(&core), &AllowAll);
    assert_eq!(ids, ["network", "network/services"]);
    // Nothing is hidden by access, so there is no "limited access" hint.
    assert!(!ids.iter().any(|id| id == "access-limited"));
}

#[test]
fn a_served_optional_kind_shows() {
    let gateway_api = ServedKinds::from_kinds(&discovery_sets::core_with_gateway_api());
    assert_eq!(
        row_ids(Some(&gateway_api), &AllowAll),
        [
            "network",
            "network/services",
            "network/gateways",
            "gateway-only",
            "gateway-only/gateways"
        ]
    );
}

#[test]
fn until_discovery_answers_nothing_is_hidden_by_presence() {
    assert_eq!(row_ids(None, &AllowAll).len(), 5);
}

#[test]
fn the_visibility_hook_hides_entries_and_says_so() {
    let gateway_api = ServedKinds::from_kinds(&discovery_sets::core_with_gateway_api());
    let ids = row_ids(Some(&gateway_api), &NoGateways);
    assert_eq!(ids, ["network", "network/services", "access-limited"]);
}

#[gpui::test]
fn the_panel_follows_discovery_and_the_hook(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_gated(cx, AccessRules::all_access(), Arc::new(NoGateways));
    fx.vcx.update(|_, cx| {
        for section in sections() {
            SidebarRegistry::register(cx, section);
        }
    });
    fx.ports.discovery.set_kinds(discovery_sets::core());
    fx.connect_following();
    let ids = fx.row_ids();
    assert!(ids.contains(&"network/services".to_owned()), "{ids:?}");
    assert!(!ids.iter().any(|id| id.ends_with("/gateways")), "{ids:?}");
    assert!(!ids.contains(&"gateway-only".to_owned()));

    // The Gateway API is installed: discovery changes and the sidebar rebuilds; the hook still
    // hides Gateways (and says so).
    fx.ports
        .discovery
        .set_kinds(discovery_sets::core_with_gateway_api());
    fx.emit(DiscoveryEvent::KindsChanged(KindsChange {
        added: vec![gateway()],
        removed: Vec::new(),
        changed: Vec::new(),
    }));
    let rows = fx.rows();
    assert!(
        rows.iter()
            .any(|r| matches!(r, Row::Notice(n) if n.id == "access-limited")),
        "{rows:?}"
    );
    assert!(!fx.row_ids().iter().any(|id| id.ends_with("/gateways")));
}

#[gpui::test]
fn an_optional_kind_appears_when_its_api_is_installed(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.vcx.update(|_, cx| {
        for section in sections() {
            SidebarRegistry::register(cx, section);
        }
    });
    fx.ports.discovery.set_kinds(discovery_sets::core());
    fx.connect_following();
    assert!(!fx.row_ids().contains(&"network/gateways".to_owned()));

    fx.ports
        .discovery
        .set_kinds(discovery_sets::core_with_gateway_api());
    fx.emit(DiscoveryEvent::KindsChanged(KindsChange {
        added: vec![gateway()],
        removed: Vec::new(),
        changed: Vec::new(),
    }));
    let ids = fx.row_ids();
    assert!(ids.contains(&"network/gateways".to_owned()), "{ids:?}");
    assert!(ids.contains(&"gateway-only/gateways".to_owned()), "{ids:?}");
}
