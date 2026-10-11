//! NetworkPolicy rule text for the tricky cases.

use oxikube_domain::view::network::{
    NetworkPolicySummary, PolicyEffect, PolicyPeer, PolicyType, Selection,
};
use serde_json::json;

use super::object;

fn policy(spec: serde_json::Value) -> NetworkPolicySummary {
    NetworkPolicySummary::from_resource(&object(
        "networking.k8s.io/v1",
        "NetworkPolicy",
        Some("db"),
        "p",
        json!({"spec": spec}),
    ))
    .expect("a policy")
}

#[test]
fn policy_types_follow_the_api_defaults() {
    assert_eq!(
        policy(json!({"podSelector": {}})).policy_types,
        [PolicyType::Ingress]
    );
    let egress_rules = policy(json!({"podSelector": {}, "egress": [{}]}));
    assert_eq!(
        egress_rules.policy_types,
        [PolicyType::Ingress, PolicyType::Egress]
    );
    let explicit = policy(json!({"podSelector": {}, "policyTypes": ["Egress"]}));
    assert_eq!(explicit.policy_types, [PolicyType::Egress]);
    assert_eq!(explicit.policy_types_text(), "Egress");
}

#[test]
fn an_empty_pod_selector_selects_every_pod_and_no_rules_is_default_deny() {
    let deny = policy(json!({"podSelector": {}, "policyTypes": ["Ingress", "Egress"]}));
    assert_eq!(deny.pod_selector, Selection::All);
    assert_eq!(deny.pod_selector_text(), "all pods");
    assert_eq!(deny.effect(PolicyType::Ingress), PolicyEffect::DenyAll);
    assert_eq!(deny.effect(PolicyType::Egress), PolicyEffect::DenyAll);

    let ingress_only = policy(json!({"podSelector": {"matchLabels": {"app": "db"}}}));
    assert_eq!(ingress_only.pod_selector_text(), "app=db");
    assert_eq!(
        ingress_only.effect(PolicyType::Egress),
        PolicyEffect::NotCovered
    );
}

#[test]
fn an_empty_rule_allows_everything() {
    let allow = policy(json!({"podSelector": {}, "ingress": [{}]}));
    assert_eq!(allow.effect(PolicyType::Ingress), PolicyEffect::AllowAll);
    let rule = &allow.ingress[0];
    assert!(rule.is_allow_all());
    assert_eq!(rule.peers_text(), "anywhere");
    assert_eq!(rule.ports_text(), "all ports");
}

#[test]
fn peers_read_as_pods_namespaces_and_ip_blocks() {
    let p = policy(json!({
        "podSelector": {"matchLabels": {"app": "db"}},
        "policyTypes": ["Ingress", "Egress"],
        "ingress": [{
            "from": [
                {"podSelector": {"matchLabels": {"app": "web"}}},
                {"podSelector": {}},
                {"namespaceSelector": {"matchLabels": {"env": "prod"}}},
                {"namespaceSelector": {}},
                {"namespaceSelector": {"matchLabels": {"env": "prod"}}, "podSelector": {"matchLabels": {"role": "api"}}},
                {"namespaceSelector": {}, "podSelector": {"matchExpressions": [{"key": "tier", "operator": "In", "values": ["a", "b"]}]}},
                {"ipBlock": {"cidr": "10.0.0.0/8", "except": ["10.1.0.0/16", "10.2.0.0/16"]}},
                {"ipBlock": {"cidr": "192.168.0.0/24"}}
            ],
            "ports": [{"port": 80}, {"protocol": "UDP", "port": 53}, {"port": "metrics"}, {"port": 8000, "endPort": 9000}, {"protocol": "SCTP"}]
        }],
        "egress": [{"to": [{"ipBlock": {"cidr": "0.0.0.0/0", "except": ["169.254.169.254/32"]}}]}]
    }));
    let rule = &p.ingress[0];
    let peers: Vec<String> = rule.peers.iter().map(ToString::to_string).collect();
    assert_eq!(
        peers,
        [
            "pods app=web in this namespace",
            "all pods in this namespace",
            "all pods in namespaces env=prod",
            "all pods in all namespaces",
            "pods role=api in namespaces env=prod",
            "pods tier in (a,b) in all namespaces",
            "IP 10.0.0.0/8 except 10.1.0.0/16, 10.2.0.0/16",
            "IP 192.168.0.0/24",
        ]
    );
    assert_eq!(
        rule.ports_text(),
        "TCP 80, UDP 53, TCP metrics, TCP 8000-9000, SCTP all ports"
    );
    assert_eq!(p.effect(PolicyType::Ingress), PolicyEffect::Restricted);
    assert_eq!(
        p.egress[0].peers_text(),
        "IP 0.0.0.0/0 except 169.254.169.254/32"
    );
    assert!(!rule.is_allow_all());
}

#[test]
fn a_peer_with_nothing_recognisable_is_kept_not_misread_as_everyone() {
    let p = policy(json!({"podSelector": {}, "ingress": [{"from": [{"bogus": 1}]}]}));
    // The rule had a `from` we could not read: it must not turn into "anywhere".
    assert_eq!(p.ingress[0].peers, [PolicyPeer::Unrecognised]);
    assert!(!p.ingress[0].is_allow_all());
    assert_eq!(p.ingress[0].peers_text(), "unrecognised peer");
}

#[test]
fn a_null_selector_is_an_absent_one() {
    let p = policy(json!({"podSelector": {}, "ingress": [{"from": [
        {"podSelector": null, "namespaceSelector": {"matchLabels": {"env": "prod"}}}
    ]}]}));
    assert_eq!(p.ingress[0].peers_text(), "all pods in namespaces env=prod");
}
