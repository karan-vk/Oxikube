//! The guard around an apply: read-only mode, the confirmation tier, the audit trail, the tool.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{self, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::{ConflictDetails, ConflictReason, OxiError};

use super::*;
use crate::command_bus::{DispatchContext, Outcome};

#[test]
fn a_read_only_cluster_is_refused_before_any_request_and_audited() {
    let t = Applying::new(true);
    let error = t.preview(t.request(CONFIGMAP)).unwrap_err();
    assert_eq!(
        error,
        ApplyError::ReadOnly {
            context: "a".into()
        }
    );
    assert!(t.calls().is_empty(), "not even a read: {:?}", t.calls());
    let audit = t.h.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Denied);
    assert_eq!(audit[0].cmd.as_ref(), "resource::Apply");
}

#[test]
fn read_only_mode_turned_on_after_the_preview_blocks_the_apply() {
    let t = Applying::new(false);
    let preview = t.preview(t.request(CONFIGMAP)).expect("preview");
    t.h.manager.set_read_only(&id("a"), true).unwrap();
    let error = t.apply(&preview).unwrap_err();
    assert!(matches!(error, ApplyError::ReadOnly { .. }), "{error:?}");
    assert_eq!(t.writes(), [("patch", true)], "only the preview's dry run");
}

#[test]
fn the_preview_is_the_confirmation_at_low_risk() {
    let meta = command::lookup(CommandId::RESOURCE_APPLY).unwrap();
    assert_eq!(meta.risk, Some(Risk::Low));
    assert_eq!(meta.confirm, ConfirmTier::Simple);
    assert!(meta.mutating);
    // The guard does ask (a simple confirm); the service answers it with the preview, so the
    // user sees one dialog: the diff.
    let t = Applying::new(false);
    let command = t.request(CONFIGMAP).command(None);
    let ctx = DispatchContext::new(Initiator::Ui, "alice").with_dry_run(true);
    let request = t.h.ask(command, ctx);
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert!(t.calls().is_empty());
}

#[test]
fn every_attempt_is_audited_with_the_object_and_never_the_manifest() {
    let t = Applying::new(false);
    let secret = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: db\nstringData:\n  password: hunter2-very-secret\n";
    let preview = t
        .preview(
            t.request(secret)
                .managed_by(Some("team-a".into()))
                .in_namespace(Some("default".into())),
        )
        .expect("preview");
    t.apply(&preview).expect("apply");
    // A failure is audited too.
    t.port()
        .script()
        .patch
        .push_err(OxiError::conflict("conflict").with_source(ConflictDetails {
            reason: ConflictReason::FieldOwnership,
            causes: Vec::new(),
        }));
    t.preview(t.request(secret).in_namespace(Some("default".into())))
        .unwrap_err();

    let audit = t.h.audit();
    let summary: Vec<(bool, AuditOutcome)> = audit.iter().map(|r| (r.dry_run, r.outcome)).collect();
    assert_eq!(
        summary,
        [
            (true, AuditOutcome::Succeeded),
            (false, AuditOutcome::Succeeded),
            (true, AuditOutcome::Failed),
        ]
    );
    for record in &audit {
        assert_eq!(record.target.gvk.kind.as_ref(), "Secret");
        assert_eq!(record.target.name.as_ref(), "db");
        assert_eq!(record.target.namespace(), Some("default"));
        assert_eq!(record.initiator, Initiator::Ui);
        let text = format!("{record:?}");
        assert!(!text.contains("hunter2"), "{text}");
    }
    assert_eq!(
        audit[0].detail.as_deref(),
        Some("manager=team-a force=false")
    );
    // A new object: no reviewed version to carry.
    assert_eq!(
        audit[1].detail.as_deref(),
        Some("manager=team-a force=false")
    );
}

#[test]
fn an_agent_dispatch_is_rehearsed_by_the_server_first() {
    let t = Applying::new(false);
    let command = t.request(CONFIGMAP).command(None);
    let ctx = DispatchContext::new(Initiator::Agent, "agent").with_cluster(id("a"));
    let outcome = t.h.confirm_and_run(command, ctx).expect("applied");
    let Outcome::Completed(output) = outcome else {
        panic!("completed")
    };
    assert_eq!(output.data.unwrap()["outcome"], "applied");
    assert_eq!(t.writes(), [("patch", true), ("patch", false)]);
    assert_eq!(t.h.audit()[0].initiator, Initiator::Agent);
}

#[test]
fn the_tool_stub_is_mutating_and_gated() {
    let t = Applying::new(false);
    let tool =
        t.h.bus
            .tool(CommandId::RESOURCE_APPLY)
            .expect("a tool stub");
    assert_eq!(tool.name.as_str(), "k8s.resource_apply");
    assert!(tool.is_mutating());
    assert_eq!(tool.risk, Some(Risk::Low));
    assert_eq!(t.h.bus.owner(CommandId::RESOURCE_APPLY), Some("test_extra"));
}
