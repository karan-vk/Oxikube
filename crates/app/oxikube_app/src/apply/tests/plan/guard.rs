//! The guard around a plan: refused whole on a read-only cluster, one confirmation at the riskiest
//! object's risk, nothing sent when it is declined, one audit record per object under one batch.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_domain::safety::{ConfirmTier, Risk};

use super::*;

#[test]
fn a_read_only_cluster_refuses_the_whole_plan_before_any_call() {
    let t = Applying::new(true);
    let plan = t.plan(vec![namespace("shop"), configmap("a", Some("shop"))]);
    let confirm = Answering::confirm();
    let error = t
        .apply_plan(&plan, &ApplyOptions::default(), confirm.clone())
        .unwrap_err();
    assert_eq!(
        error,
        ApplyError::ReadOnly {
            context: "a".into()
        }
    );
    let error = t.dry_run_plan(&plan, &ApplyOptions::default()).unwrap_err();
    assert!(matches!(error, ApplyError::ReadOnly { .. }), "{error:?}");
    assert!(t.calls().is_empty(), "not even a read: {:?}", t.calls());
    assert!(confirm.asked.lock().is_empty(), "nothing to confirm");
    let audit = t.h.audit();
    assert_eq!(
        audit.len(),
        2,
        "one denied record per attempt, not per object"
    );
    assert!(
        audit
            .iter()
            .all(|r| r.outcome == AuditOutcome::Denied && r.cmd.as_ref() == "resource::ApplyPlan")
    );
}

#[test]
fn a_declined_confirmation_sends_nothing() {
    let t = Applying::new(false);
    let plan = t.plan(vec![configmap("a", None), configmap("b", None)]);
    let confirm = Answering::decline();
    let error = t
        .apply_plan(&plan, &ApplyOptions::default(), confirm.clone())
        .unwrap_err();
    assert_eq!(error, ApplyError::Declined);
    assert_eq!(confirm.asked.lock().len(), 1);
    assert!(t.calls().is_empty(), "no read, no write: {:?}", t.calls());
    let audit = t.h.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Cancelled);
}

#[test]
fn one_confirmation_for_the_whole_plan_at_the_riskiest_objects_risk() {
    let t = Applying::new(false);
    // A ConfigMap alone is low; the Namespace (cluster-scoped) and the RBAC Role raise the plan.
    let role = json!({"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
        "metadata": {"name": "reader"}});
    let plan = t.plan(vec![
        configmap("a", None),
        configmap("b", None),
        namespace("shop"),
        role,
    ]);
    let confirm = Answering::confirm();
    t.apply_plan(&plan, &ApplyOptions::default(), confirm.clone())
        .expect("applied");
    let asked = confirm.asked.lock();
    assert_eq!(asked.len(), 1, "one confirmation, not one per object");
    let request = &asked[0];
    assert_eq!(request.command, CommandId::RESOURCE_APPLY_PLAN);
    assert_eq!(request.risk, Some(Risk::Medium));
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert_eq!(request.initiator, Initiator::Ui);
    assert!(
        request
            .summary
            .starts_with("Apply 4 objects on a: 2 ConfigMap, 1 Namespace, 1 Role."),
        "{}",
        request.summary
    );

    let low = t.plan(vec![configmap("c", None)]);
    let confirm = Answering::confirm();
    t.apply_plan(&low, &ApplyOptions::default(), confirm.clone())
        .expect("applied");
    assert_eq!(confirm.asked.lock()[0].risk, Some(Risk::Low));

    let forced = ApplyOptions {
        force_conflicts: true,
        ..ApplyOptions::default()
    };
    let confirm = Answering::confirm();
    t.apply_plan(&low, &forced, confirm.clone())
        .expect("applied");
    let request = &confirm.asked.lock()[0];
    assert_eq!(request.risk, Some(Risk::Medium), "force raises it");
    assert!(request.summary.contains("(force)"), "{}", request.summary);
}

#[test]
fn every_object_is_audited_once_under_the_plans_batch() {
    let t = Applying::new(false);
    let plan = t.plan(vec![namespace("shop"), configmap("a", Some("shop"))]);
    t.apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("applied");
    let audit = t.h.audit();
    let objects: Vec<_> = audit
        .iter()
        .filter(|r| r.cmd.as_ref() == "resource::Apply")
        .collect();
    assert_eq!(objects.len(), 2, "one record per object: {audit:#?}");
    let plan_record = audit
        .iter()
        .find(|r| r.cmd.as_ref() == "resource::ApplyPlan")
        .expect("the plan's own record");
    let batch = plan_record
        .batch_id
        .clone()
        .expect("the plan names its batch");
    assert!(batch.starts_with("apply-"));
    for record in &objects {
        assert_eq!(record.batch_id.as_deref(), Some(&*batch));
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
        assert!(!record.dry_run);
        assert_eq!(record.initiator, Initiator::Ui);
        let detail = record.detail.as_deref().unwrap();
        assert!(
            detail.contains("via=resource::ApplyPlan") && detail.contains("manager=oxikube"),
            "{detail}"
        );
    }
    assert_eq!(objects[0].target.name.as_ref(), "shop");
    assert_eq!(objects[1].target.namespace(), Some("shop"));
    let detail = plan_record.detail.as_deref().unwrap();
    assert!(
        detail.contains("items=2") && detail.contains("created=2"),
        "{detail}"
    );
}

#[test]
fn the_dry_run_is_audited_per_object_as_a_dry_run() {
    let t = Applying::new(false);
    let plan = t.plan(vec![configmap("a", None), configmap("b", None)]);
    t.dry_run_plan(&plan, &ApplyOptions::default())
        .expect("dry run");
    let audit = t.h.audit();
    let objects: Vec<_> = audit
        .iter()
        .filter(|r| r.cmd.as_ref() == "resource::Apply")
        .collect();
    assert_eq!(objects.len(), 2);
    assert!(objects.iter().all(|r| r.dry_run));
}

#[test]
fn a_secret_never_reaches_a_result_a_report_or_the_audit_log() {
    let t = Applying::new(false);
    let secret = json!({"apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "db"}, "stringData": {"password": "hunter2-very-secret"}});
    let plan = t.plan(vec![secret]);
    assert!(!format!("{plan:?}").contains("hunter2"));
    let dry = t
        .dry_run_plan(&plan, &ApplyOptions::default())
        .expect("dry run");
    let applied = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("applied");
    for report in [dry, applied] {
        let text = serde_json::to_string(&report).unwrap();
        assert!(!text.contains("hunter2"), "{text}");
    }
    let audit = format!("{:?}", t.h.audit());
    assert!(!audit.contains("hunter2"), "{audit}");
}

#[test]
fn read_only_mode_turned_on_after_the_dry_run_blocks_the_apply() {
    let t = Applying::new(false);
    let plan = t.plan(vec![configmap("a", None)]);
    t.dry_run_plan(&plan, &ApplyOptions::default())
        .expect("dry run");
    t.h.manager.set_read_only(&id("a"), true).unwrap();
    let error = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .unwrap_err();
    assert!(matches!(error, ApplyError::ReadOnly { .. }), "{error:?}");
    assert_eq!(t.writes(), [("patch", true)], "only the dry run");
}
