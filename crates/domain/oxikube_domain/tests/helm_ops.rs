//! E16-S06: `helm::Rollback` and `helm::Uninstall` are guarded mutations with tool stubs.

use oxikube_domain::command::{Command, CommandId, lookup};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::safety::{ConfirmTier, Risk};
use serde_json::json;

#[test]
fn rollback_is_medium_with_a_simple_confirmation() {
    let meta = lookup(CommandId::HELM_ROLLBACK).expect("declared");
    assert!(meta.mutating);
    assert_eq!(meta.risk, Some(Risk::Medium));
    assert_eq!(meta.confirm, ConfirmTier::Simple);
    assert_eq!(CommandId::HELM_ROLLBACK.tool_name(), "helm.rollback");
}

#[test]
fn uninstall_is_high_and_types_the_name() {
    let meta = lookup(CommandId::HELM_UNINSTALL).expect("declared");
    assert!(meta.mutating);
    assert_eq!(meta.risk, Some(Risk::High));
    assert_eq!(meta.risk.unwrap().confirm_tier(), ConfirmTier::TypeName);
    assert_eq!(CommandId::HELM_UNINSTALL.tool_name(), "helm.uninstall");
}

#[test]
fn a_tool_call_round_trips() {
    let cluster = ClusterId::new("/k", &ContextName::new("kind"));
    let command = Command::HelmUninstall {
        cluster,
        namespace: "demo".into(),
        release: "web".into(),
        keep_history: true,
    };
    assert!(command.is_mutating());
    let back: Command = serde_json::from_value(serde_json::to_value(&command).unwrap()).unwrap();
    assert_eq!(back, command);
    let minimal: Command = serde_json::from_value(json!({
        "type": "helm::Rollback",
        "cluster": serde_json::to_value(ClusterId::new("/k", &ContextName::new("kind"))).unwrap(),
        "namespace": "demo", "release": "web", "revision": 2,
    }))
    .unwrap();
    assert_eq!(minimal.id(), CommandId::HELM_ROLLBACK);
}
