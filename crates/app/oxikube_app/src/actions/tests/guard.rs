//! `resource::Delete` through the guard: tiers, propagation, read-only, audit, the tool stub.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{CommandId, Propagation};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_ports::{DeleteOutcome, PropagationPolicy};
use oxikube_testkit::ResourceCall;

use super::{cluster_scoped, delete, namespaced};
use crate::command_bus::{DispatchError, Outcome};
use crate::guard::{Confirmation, ConfirmationError};
use crate::testing::{Harness, ctx};

fn pod(name: &str) -> oxikube_domain::ids::ResourceRef {
    namespaced("a", "", "Pod", name)
}

fn deletes(h: &Harness) -> Vec<(bool, Option<PropagationPolicy>)> {
    h.writes("a")
        .into_iter()
        .map(|call| match call {
            ResourceCall::Delete { options, .. } => (options.dry_run, options.propagation),
            other => panic!("unexpected write {other:?}"),
        })
        .collect()
}

#[test]
fn a_pod_takes_a_simple_confirm_then_a_dry_run_and_the_delete() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    h.allow_deletes("a", 2);
    let cmd = delete(pod("web-0"), Propagation::Background);

    let request = h.ask(cmd.clone(), ctx(Initiator::Ui));
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert_eq!(request.risk, Some(Risk::Medium));
    assert_eq!(request.expected_name, None);
    assert!(
        request
            .summary
            .starts_with("Delete Resource: Pod default/web-0")
    );
    assert!(
        h.writes("a").is_empty(),
        "nothing is sent before the answer"
    );

    let out = h
        .dispatch(
            cmd,
            ctx(Initiator::Ui).with_confirmation(Confirmation::simple(request.token)),
        )
        .unwrap();
    let Outcome::Completed(output) = out else {
        panic!("expected completion");
    };
    assert_eq!(output.message.as_deref(), Some("Deleted Pod default/web-0"));
    // Server dry run first, then the real delete, with kubectl's default propagation.
    assert_eq!(
        deletes(&h),
        [
            (true, Some(PropagationPolicy::Background)),
            (false, Some(PropagationPolicy::Background))
        ]
    );
    assert_eq!(h.outcomes(), [(AuditOutcome::Succeeded, Initiator::Ui)]);
    let audit = h.audit();
    assert_eq!(&*audit[0].cmd, "resource::Delete");
    assert_eq!(audit[0].target, pod("web-0"));
}

#[test]
fn a_finalizer_that_keeps_the_object_is_reported_as_deleting() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    let resources = h.resources("a");
    resources.script().delete.push_ok(DeleteOutcome::Deleted);
    resources.script().delete.push_ok(DeleteOutcome::Deleting(
        oxikube_domain::Resource::from_json(serde_json::json!({
            "apiVersion": "v1", "kind": "Pod", "metadata": {"name": "web-0", "namespace": "default"}
        }))
        .unwrap(),
    ));
    let out = h
        .confirm_and_run(
            delete(pod("web-0"), Propagation::Background),
            ctx(Initiator::Ui),
        )
        .unwrap();
    let Outcome::Completed(output) = out else {
        panic!("expected completion");
    };
    assert_eq!(output.data.unwrap()["outcome"], "deleting");
}

#[test]
fn the_propagation_choice_reaches_the_request() {
    for (propagation, policy) in [
        (Propagation::Background, PropagationPolicy::Background),
        (Propagation::Foreground, PropagationPolicy::Foreground),
        (Propagation::Orphan, PropagationPolicy::Orphan),
    ] {
        let h = Harness::with_delete_handler();
        h.connect("a", false);
        h.allow_deletes("a", 2);
        let target = namespaced("a", "apps", "Deployment", "api");
        h.confirm_and_run(delete(target, propagation), ctx(Initiator::Ui))
            .unwrap();
        assert_eq!(deletes(&h), [(true, Some(policy)), (false, Some(policy))]);
    }
}

#[test]
fn namespaces_nodes_and_volumes_need_the_name_typed() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    for (kind, name, risk) in [
        ("Namespace", "payments", Risk::Irreversible),
        ("Node", "worker-1", Risk::High),
        ("PersistentVolume", "pv-data", Risk::Irreversible),
    ] {
        let request = h.ask(
            delete(cluster_scoped("a", kind, name), Propagation::Background),
            ctx(Initiator::Ui),
        );
        assert_eq!(request.tier, ConfirmTier::TypeName, "{kind}");
        assert_eq!(request.expected_name.as_deref(), Some(name), "{kind}");
        assert_eq!(request.risk, Some(risk), "{kind}");
    }
}

#[test]
fn a_cascading_delete_of_an_ordinary_object_needs_the_name_typed() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    let request = h.ask(
        delete(
            namespaced("a", "apps", "Deployment", "api"),
            Propagation::Foreground,
        ),
        ctx(Initiator::Ui),
    );
    assert_eq!(request.tier, ConfirmTier::TypeName);
    assert_eq!(request.expected_name.as_deref(), Some("api"));
    assert_eq!(request.risk, Some(Risk::High));
}

#[test]
fn a_wrong_name_deletes_nothing() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    h.allow_deletes("a", 2);
    let cmd = delete(
        cluster_scoped("a", "Namespace", "payments"),
        Propagation::Background,
    );
    let request = h.ask(cmd.clone(), ctx(Initiator::Ui));
    let err = h
        .dispatch(
            cmd.clone(),
            ctx(Initiator::Ui).with_confirmation(Confirmation::typed(request.token, "payment")),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        DispatchError::Confirmation(ConfirmationError::NameMismatch { .. })
    ));
    assert!(h.writes("a").is_empty());
    assert_eq!(h.outcomes(), [(AuditOutcome::Denied, Initiator::Ui)]);

    // The right name goes through.
    let request = h.ask(cmd.clone(), ctx(Initiator::Ui));
    h.dispatch(
        cmd,
        ctx(Initiator::Ui).with_confirmation(Confirmation::typed(request.token, "payments")),
    )
    .unwrap();
    assert_eq!(h.writes("a").len(), 2);
}

#[test]
fn a_read_only_cluster_refuses_every_initiator_before_any_request() {
    let h = Harness::with_delete_handler();
    h.connect("a", true);
    for initiator in Initiator::ALL {
        let err = h
            .dispatch(
                delete(pod("web-0"), Propagation::Background),
                ctx(initiator),
            )
            .unwrap_err();
        assert!(matches!(err, DispatchError::ReadOnly { .. }), "{initiator}");
    }
    assert!(h.writes("a").is_empty());
    assert_eq!(h.audit().len(), Initiator::ALL.len());
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn a_dry_run_dispatch_only_rehearses() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    h.allow_deletes("a", 1);
    let out = h
        .confirm_and_run(
            delete(pod("web-0"), Propagation::Background),
            ctx(Initiator::Command).with_dry_run(true),
        )
        .unwrap();
    assert!(matches!(out, Outcome::Completed(_)));
    assert_eq!(deletes(&h), [(true, Some(PropagationPolicy::Background))]);
    assert!(h.audit()[0].dry_run);
}

#[test]
fn a_server_refusal_in_the_rehearsal_stops_before_the_delete() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    h.resources("a")
        .script()
        .delete
        .push_err(oxikube_domain::OxiError::forbidden(
            "pods web-0 is forbidden",
        ));
    let err = h
        .confirm_and_run(
            delete(pod("web-0"), Propagation::Background),
            ctx(Initiator::Ui),
        )
        .unwrap_err();
    assert!(matches!(err, DispatchError::Handler(_)));
    assert_eq!(deletes(&h), [(true, Some(PropagationPolicy::Background))]);
    assert_eq!(h.outcomes(), [(AuditOutcome::Failed, Initiator::Ui)]);
}

#[test]
fn delete_has_an_mcp_tool_stub_that_needs_mutate() {
    let h = Harness::with_delete_handler();
    let tool = h.bus.tool(CommandId::RESOURCE_DELETE).expect("a stub");
    assert_eq!(tool.name.as_str(), "k8s.resource_delete");
    assert!(tool.needs.contains(oxikube_domain::Capabilities::MUTATE));
    assert_eq!(h.bus.owner(CommandId::RESOURCE_DELETE), Some("test_extra"));
}
