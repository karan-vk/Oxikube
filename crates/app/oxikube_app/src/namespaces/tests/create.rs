//! `namespace::Create`: low risk, one confirm, a server dry run then the create, audited.

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_testkit::ResourceCall;

use super::{create, harness};
use crate::command_bus::{DispatchError, Outcome};
use crate::testing::ctx;

fn creates(h: &crate::testing::Harness) -> Vec<(bool, String)> {
    h.writes("a")
        .into_iter()
        .map(|call| match call {
            ResourceCall::Create {
                options, object, ..
            } => (
                options.dry_run,
                object["metadata"]["name"].as_str().unwrap().to_owned(),
            ),
            other => panic!("unexpected write {other:?}"),
        })
        .collect()
}

#[test]
fn the_command_is_a_low_risk_mutation_with_a_tool_stub() {
    let meta = oxikube_domain::command::lookup(CommandId::NAMESPACE_CREATE).unwrap();
    assert!(meta.mutating);
    assert_eq!(meta.risk, Some(Risk::Low));
    assert_eq!(meta.confirm, ConfirmTier::Simple);
    assert_eq!(
        CommandId::NAMESPACE_CREATE.tool_name(),
        "app.namespace_create"
    );
}

#[test]
fn one_confirm_then_a_dry_run_and_the_create() {
    let h = harness();
    h.connect("a", false);
    let command = create("a", "team-a", &[("team", "a")]);

    let request = h.ask(command.clone(), ctx(Initiator::Ui));
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert_eq!(request.risk, Some(Risk::Low));
    // Only the guard's dry run is sent before the answer (E19-S07); the question previews it.
    assert_eq!(
        creates(&h),
        [(true, "team-a".to_owned())],
        "nothing is sent before the answer"
    );
    let Some(crate::guard::DryRunPreview::Rehearsed(preview)) = &request.preview else {
        panic!("a create is previewed: {request:?}");
    };
    assert_eq!(&*preview.changes[0].target.name, "team-a");

    let answer = crate::guard::Confirmation::simple(request.token);
    let out = h
        .dispatch(command, ctx(Initiator::Ui).with_confirmation(answer))
        .unwrap();
    let Outcome::Completed(output) = out else {
        panic!("expected completion");
    };
    assert_eq!(output.message.as_deref(), Some("Created namespace team-a"));
    assert_eq!(
        creates(&h),
        [
            (true, "team-a".to_owned()),
            (true, "team-a".to_owned()),
            (false, "team-a".to_owned())
        ]
    );
    // The object that was sent has the labels.
    let ResourceCall::Create { object, .. } = &h.writes("a")[2] else {
        panic!("a create");
    };
    assert_eq!(object["metadata"]["labels"]["team"], "a");

    // The guard's dry run, then the create, naming the new namespace (not the cluster), with the
    // label count.
    let audit = h.audit();
    assert_eq!(audit.len(), 2);
    assert!(audit[0].dry_run);
    let audit = &audit[1..];
    assert!(!audit[0].dry_run);
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(&*audit[0].cmd, "namespace::Create");
    assert_eq!(&*audit[0].target.name, "team-a");
    assert_eq!(&*audit[0].target.gvk.kind, "Namespace");
    assert_eq!(audit[0].detail.as_deref(), Some("labels=1"));
}

#[test]
fn a_read_only_cluster_refuses_it_for_everyone_and_audits_the_refusal() {
    let h = harness();
    h.connect("a", true);
    for initiator in [Initiator::Ui, Initiator::Command, Initiator::Agent] {
        let err = h
            .dispatch(create("a", "team-a", &[]), ctx(initiator))
            .unwrap_err();
        assert!(
            matches!(err, DispatchError::ReadOnly { .. }),
            "{initiator:?}"
        );
    }
    assert!(h.writes("a").is_empty(), "no request left");
    assert_eq!(h.audit().len(), 3);
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn an_invalid_name_or_label_is_refused_before_any_request() {
    let h = harness();
    h.connect("a", false);
    for command in [
        create("a", "Team A", &[]),
        create("a", "team-a", &[("bad key", "v")]),
        create("a", "team-a", &[("team", "-bad")]),
    ] {
        let err = h.confirm_and_run(command, ctx(Initiator::Ui)).unwrap_err();
        let DispatchError::Handler(err) = err else {
            panic!("a handler error: {err:?}");
        };
        assert_eq!(err.kind(), ErrorKind::Validation);
    }
    assert!(h.writes("a").is_empty());
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Failed));
}

#[test]
fn a_name_that_exists_fails_in_the_dry_run_and_creates_nothing() {
    let h = harness();
    h.connect("a", false);
    let existing = oxikube_domain::Resource::from_json(serde_json::json!({
        "apiVersion": "v1", "kind": "Namespace", "metadata": { "name": "team-a" }
    }))
    .unwrap();
    h.resources("a").insert(existing);
    let err = h
        .confirm_and_run(create("a", "team-a", &[]), ctx(Initiator::Ui))
        .unwrap_err();
    let DispatchError::Handler(err) = err else {
        panic!("a handler error");
    };
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(
        creates(&h),
        [(true, "team-a".to_owned())],
        "only the rehearsal"
    );
    assert_eq!(h.outcomes(), [(AuditOutcome::Failed, Initiator::Ui)]);
}

#[test]
fn a_dry_run_dispatch_stops_after_the_rehearsal() {
    let h = harness();
    h.connect("a", false);
    let out = h
        .confirm_and_run(
            create("a", "team-a", &[]),
            ctx(Initiator::Command).with_dry_run(true),
        )
        .unwrap();
    let Outcome::Completed(output) = out else {
        panic!("expected completion");
    };
    assert_eq!(output.data.unwrap()["outcome"], "dry_run");
    assert_eq!(creates(&h), [(true, "team-a".to_owned())]);
}

#[test]
fn the_creator_dispatches_as_the_user_and_answers_the_one_click_confirm() {
    let h = harness();
    h.connect("a", false);
    let creator = crate::namespaces::NamespaceCreator::new(h.bus.clone(), "alice");
    let labels = std::collections::BTreeMap::from([("team".to_owned(), "a".to_owned())]);

    let message =
        futures::executor::block_on(creator.create(&crate::testing::id("a"), "team-a", labels))
            .unwrap();

    assert_eq!(message, "Created namespace team-a");
    assert_eq!(
        creates(&h),
        [
            (true, "team-a".to_owned()),
            (true, "team-a".to_owned()),
            (false, "team-a".to_owned())
        ]
    );
    // The guard's dry run, then the create.
    assert_eq!(
        h.outcomes(),
        [
            (AuditOutcome::Succeeded, Initiator::Ui),
            (AuditOutcome::Succeeded, Initiator::Ui)
        ]
    );
}

#[test]
fn the_creator_checks_the_fields_before_it_asks_anything() {
    let h = harness();
    h.connect("a", false);
    let creator = crate::namespaces::NamespaceCreator::new(h.bus.clone(), "alice");
    let err = futures::executor::block_on(creator.create(
        &crate::testing::id("a"),
        "Not Valid",
        Default::default(),
    ))
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(h.writes("a").is_empty());
    assert!(h.audit().is_empty(), "nothing reached the guard");
}

#[test]
fn the_creator_reports_a_read_only_cluster_as_forbidden() {
    let h = harness();
    h.connect("a", true);
    let creator = crate::namespaces::NamespaceCreator::new(h.bus.clone(), "alice");
    let err = futures::executor::block_on(creator.create(
        &crate::testing::id("a"),
        "team-a",
        Default::default(),
    ))
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(h.writes("a").is_empty());
}

#[test]
fn a_server_that_cannot_dry_run_still_gets_the_confirmed_create() {
    let h = harness();
    h.connect("a", false);
    let output = h.confirm_without_dry_runs("a", create("a", "team-a", &[]), ctx(Initiator::Ui));
    assert_eq!(output.message.as_deref(), Some("Created namespace team-a"));
    assert_eq!(
        creates(&h),
        [(true, "team-a".to_owned()), (false, "team-a".to_owned())]
    );
}
