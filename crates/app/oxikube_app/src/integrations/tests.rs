use std::sync::Arc;

use oxikube_domain::Capabilities;
use oxikube_domain::command::CommandId;
use oxikube_ports::{SidebarItem, SidebarModel, SidebarSection};
use oxikube_testkit::FakeIntegrationPort;

use super::{IntegrationRegistry, RegisterIntegrationError};

fn section(id: &str, needs: Capabilities) -> SidebarSection {
    SidebarSection {
        id: id.into(),
        title: id.to_uppercase(),
        icon: None,
        items: vec![SidebarItem {
            id: format!("{id}-list"),
            title: "List".into(),
            icon: None,
            command: CommandId::new("argo::Open"),
            needs,
        }],
    }
}

fn integration(id: &str, sections: Vec<SidebarSection>) -> Arc<FakeIntegrationPort> {
    Arc::new(FakeIntegrationPort::new(id).with_sidebar(SidebarModel { sections }))
}

#[test]
fn sections_come_in_registration_order_and_only_when_the_cluster_offers_them() {
    let registry = IntegrationRegistry::new();
    registry
        .register(integration(
            "argocd",
            vec![section("apps", Capabilities::ARGO)],
        ))
        .unwrap();
    registry
        .register(integration(
            "flux",
            vec![section("kustomizations", Capabilities::empty())],
        ))
        .unwrap();
    assert_eq!(registry.ids(), ["argocd", "flux"]);

    let all = registry.sidebar_sections(Capabilities::ARGO);
    let keys: Vec<_> = all
        .iter()
        .map(|s| (s.integration.as_str(), s.section.id.as_str()))
        .collect();
    assert_eq!(keys, [("argocd", "apps"), ("flux", "kustomizations")]);

    // Without Argo CD on the cluster its section goes away; the other stays.
    let without = registry.sidebar_sections(Capabilities::empty());
    assert_eq!(without.len(), 1);
    assert_eq!(without[0].integration, "flux");
}

#[test]
fn an_id_registers_once() {
    let registry = IntegrationRegistry::new();
    registry.register(integration("argocd", vec![])).unwrap();
    let err = registry
        .register(integration("argocd", vec![]))
        .unwrap_err();
    assert_eq!(err, RegisterIntegrationError::Duplicate("argocd".into()));
    assert_eq!(registry.ids().len(), 1);
}

#[test]
fn clones_share_registrations() {
    let registry = IntegrationRegistry::new();
    let clone = registry.clone();
    clone
        .register(integration(
            "argocd",
            vec![section("apps", Capabilities::empty())],
        ))
        .unwrap();
    assert_eq!(registry.sidebar_sections(Capabilities::empty()).len(), 1);
}
