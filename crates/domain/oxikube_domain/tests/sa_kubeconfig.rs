//! E17-S09: `serviceaccount::GenerateKubeconfig` is a guarded, low-risk mutation with a tool stub.

use oxikube_domain::command::{Command, CommandId, lookup};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::safety::{ConfirmTier, Risk};
use serde_json::json;

fn account() -> ResourceRef {
    ResourceRef::namespaced(
        ClusterId::new("/k", &ContextName::new("kind")),
        Gvk::new("", "v1", "ServiceAccount"),
        "demo",
        "deployer",
    )
}

#[test]
fn generating_a_kubeconfig_is_low_risk_with_a_simple_confirmation() {
    let id = CommandId::SERVICEACCOUNT_GENERATE_KUBECONFIG;
    let meta = lookup(id).expect("declared");
    assert!(meta.mutating, "it creates a credential");
    assert_eq!(meta.risk, Some(Risk::Low));
    assert_eq!(meta.confirm, ConfirmTier::Simple);
    assert!(meta.availability.requires_writable);
    assert_eq!(id.tool_name(), "k8s.serviceaccount_generate_kubeconfig");
}

#[test]
fn a_tool_call_round_trips_and_defaults_its_options() {
    let command = Command::ServiceAccountGenerateKubeconfig {
        target: account(),
        expiration_seconds: Some(600),
        audiences: vec!["https://example.com".into()],
        allow_legacy_token: true,
    };
    assert!(command.is_mutating());
    let back: Command = serde_json::from_value(serde_json::to_value(&command).unwrap()).unwrap();
    assert_eq!(back, command);
    let minimal: Command = serde_json::from_value(json!({
        "type": "serviceaccount::GenerateKubeconfig",
        "target": serde_json::to_value(account()).unwrap(),
    }))
    .unwrap();
    assert_eq!(
        minimal,
        Command::ServiceAccountGenerateKubeconfig {
            target: account(),
            expiration_seconds: None,
            audiences: Vec::new(),
            allow_legacy_token: false,
        }
    );
}
