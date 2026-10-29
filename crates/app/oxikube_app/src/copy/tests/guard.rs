//! The target's guard around a copy: read-only, confirmation tiers, a declined confirmation.

use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::safety::ConfirmTier;

use super::*;
use crate::guard::{ConfirmationRequest, Decision};

struct Declining;

#[async_trait::async_trait]
impl ConfirmationProvider for Declining {
    async fn confirm(&self, _: &ConfirmationRequest) -> Decision {
        Decision::Decline
    }
}

#[test]
fn a_read_only_target_stops_the_flow_at_the_start_with_zero_reads_and_zero_writes() {
    let t = Copying::new(true);
    let error = t.preview(t.request(&configmap(), "prod")).unwrap_err();
    assert_eq!(
        error,
        CopyError::ReadOnly {
            context: "b".into()
        }
    );
    assert!(t.h.writes("b").is_empty());
    assert_eq!(t.source_writes(), 0);
    // The source was not even read: the guard refused before the handler ran.
    assert!(
        !t.source()
            .recorded_calls()
            .iter()
            .any(|call| matches!(call, ResourceCall::Get { .. } | ResourceCall::GetOpt { .. })),
        "{:?}",
        t.source().recorded_calls()
    );
    let audit = t.h.audit();
    let denied: Vec<_> = audit
        .iter()
        .filter(|r| r.outcome == AuditOutcome::Denied)
        .collect();
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0].cmd.as_ref(), "resource::CopyTo");
    assert_eq!(denied[0].cluster, id("b"));
}

#[test]
fn a_read_only_source_does_not_block_the_copy() {
    let t = Copying::new(false);
    t.h.manager.set_read_only(&id("a"), true).unwrap();
    let preview = t.preview(t.request(&configmap(), "prod")).expect("preview");
    t.copy(&preview, None).expect("copy");
    assert_eq!(t.target_writes().len(), 1);
    assert_eq!(t.source_writes(), 0);
}

#[test]
fn read_only_mode_turned_on_after_the_review_blocks_the_copy() {
    let t = Copying::new(false);
    let preview = t.preview(t.request(&configmap(), "prod")).expect("preview");
    t.h.manager.set_read_only(&id("b"), true).unwrap();
    let error = t.copy(&preview, None).unwrap_err();
    assert!(matches!(error, CopyError::ReadOnly { .. }), "{error:?}");
    assert!(t.target_writes().is_empty());
}

#[test]
fn the_tier_follows_the_target_not_the_source() {
    let t = Copying::new(false);
    let tier = |object: &Value| t.preview(t.request(object, "prod")).expect("preview").tier;
    assert_eq!(tier(&configmap()), ConfirmTier::Simple);
    let secret = json!({"apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "s", "namespace": "shop"}, "data": {}});
    assert_eq!(tier(&secret), ConfirmTier::Simple);
    let role = json!({"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
        "metadata": {"name": "r", "namespace": "shop"}, "rules": []});
    assert_eq!(tier(&role), ConfirmTier::Simple);
    let namespace = json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": "shop"}});
    let preview = t
        .preview(CopyRequest::to(source_ref(&namespace), id("b"), None))
        .expect("preview");
    assert_eq!(preview.tier, ConfirmTier::TypeName);
}

#[test]
fn a_cluster_scoped_copy_needs_the_typed_name() {
    let t = Copying::new(false);
    let namespace = json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": "shop"}});
    let preview = t
        .preview(CopyRequest::to(source_ref(&namespace), id("b"), None))
        .expect("preview");
    // The review's click alone is not the typed name.
    let error = t
        .copy_with(&preview, None, ReviewAnswer::click())
        .unwrap_err();
    assert_eq!(error, CopyError::Declined);
    assert!(t.target_writes().is_empty());
    // A wrong name is refused by the guard; the right one goes through.
    assert!(
        t.copy_with(&preview, None, ReviewAnswer::typed("nope"))
            .is_err()
    );
    assert!(t.target_writes().is_empty());
    t.copy_with(&preview, None, ReviewAnswer::typed("shop"))
        .expect("copy");
    assert_eq!(t.target_writes(), [("patch", "shop".to_owned())]);
}

#[test]
fn a_declined_confirmation_writes_nothing() {
    let t = Copying::new(false);
    let preview = t.preview(t.request(&configmap(), "prod")).expect("preview");
    let error = t
        .copy_with(&preview, None, Arc::new(Declining))
        .unwrap_err();
    assert_eq!(error, CopyError::Declined);
    assert!(t.target_writes().is_empty());
    let cancelled =
        t.h.audit()
            .iter()
            .filter(|r| r.outcome == AuditOutcome::Cancelled)
            .count();
    assert_eq!(cancelled, 1);
}

#[test]
fn the_tool_is_a_mutating_stub_with_the_worst_case_risk() {
    use oxikube_domain::command::{self, CommandId};
    let meta = command::lookup(CommandId::RESOURCE_COPY_TO).unwrap();
    assert!(meta.mutating);
    assert_eq!(
        CommandId::RESOURCE_COPY_TO.tool_name(),
        "k8s.resource_copy_to"
    );
}
