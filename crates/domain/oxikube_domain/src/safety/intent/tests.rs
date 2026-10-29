use super::*;
use crate::ids::ContextName;

fn cluster() -> ClusterId {
    ClusterId::new("kubeconfig", &ContextName::new("prod"))
}

fn pod() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "default", "web-0")
}

#[test]
fn risk_comes_from_classify() {
    let intent = MutationIntent::new(
        &pod(),
        MutationVerb::Delete,
        MutationFlags::default(),
        Initiator::Ui,
    );
    assert_eq!(intent.risk, Risk::Medium);
    let namespace = ResourceRef::cluster_scoped(cluster(), Gvk::new("", "v1", "Namespace"), "dev");
    let intent = MutationIntent::new(
        &namespace,
        MutationVerb::Delete,
        MutationFlags::default(),
        Initiator::Agent,
    );
    assert_eq!(intent.risk, Risk::Irreversible);
    assert_eq!(intent.namespace, None);
    assert_eq!(intent.target(), namespace);
}

#[test]
fn target_round_trips() {
    let intent = MutationIntent::new(
        &pod(),
        MutationVerb::Patch,
        MutationFlags::default(),
        Initiator::Command,
    );
    assert_eq!(intent.target(), pod());
}

#[test]
fn serde_round_trip_with_stable_keys_and_no_body() {
    let intent = MutationIntent::new(
        &pod(),
        MutationVerb::DeleteCollection,
        MutationFlags::default(),
        Initiator::Plugin,
    )
    .in_batch("b1", 12);
    let json = serde_json::to_value(&intent).unwrap();
    let mut keys: Vec<_> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "batch_id",
            "cluster",
            "count",
            "gvk",
            "initiator",
            "name",
            "namespace",
            "risk",
            "verb"
        ]
    );
    assert_eq!(json["verb"], "delete_collection");
    assert_eq!(json["risk"], "high");
    for banned in ["body", "manifest", "patch", "data", "payload"] {
        assert!(json.get(banned).is_none(), "{banned}");
    }
    assert_eq!(
        serde_json::from_value::<MutationIntent>(json).unwrap(),
        intent
    );
    // A single mutation omits the batch fields.
    let single = serde_json::to_value(MutationIntent::new(
        &pod(),
        MutationVerb::Scale,
        MutationFlags::default(),
        Initiator::Ui,
    ))
    .unwrap();
    assert!(single.get("batch_id").is_none() && single.get("count").is_none());
}

#[test]
fn verb_names_match_serde() {
    for verb in MutationVerb::ALL {
        assert_eq!(serde_json::to_string(&verb).unwrap(), format!("\"{verb}\""));
    }
}

#[test]
fn audit_record_carries_the_initiator_target_and_batch() {
    let ts: Timestamp = "2026-10-30T12:00:00Z".parse().unwrap();
    let intent = MutationIntent::new(
        &pod(),
        MutationVerb::Delete,
        MutationFlags::default(),
        Initiator::Agent,
    )
    .in_batch("batch-1", 3);
    let record = intent.audit_record(
        ts,
        "claude",
        "selection::Delete",
        false,
        AuditOutcome::Succeeded,
    );
    assert_eq!(record.initiator, Initiator::Agent);
    assert_eq!(record.target, pod());
    assert_eq!(record.batch_id.as_deref(), Some("batch-1"));
    assert_eq!(record.verb, Some(MutationVerb::Delete));
    let single = MutationIntent::new(
        &pod(),
        MutationVerb::Delete,
        MutationFlags::default(),
        Initiator::Ui,
    )
    .audit_record(ts, "me", "pod::Delete", true, AuditOutcome::Denied);
    assert_eq!(single.batch_id, None);
}
